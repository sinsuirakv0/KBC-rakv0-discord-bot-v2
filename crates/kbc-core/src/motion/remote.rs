//! LINEからの代行依頼を有限に保持し、既存TaskRuntimeへ接続する。

use super::{MotionJob, MotionPlan, request::MotionFormat};
use crate::{
    commands::common::remote_data::RemoteAssetSource,
    services::HttpService,
    task_runtime::{
        ExternalOutcome, MAX_ATTACHMENT_BYTES, TaskArtifact, TaskRuntime, cleanup_task_workspace,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const RESULT_CAPACITY: usize = 2;
const RESULT_TTL: Duration = Duration::from_secs(600);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MotionRenderRequest {
    protocol_version: u32,
    request_id: String,
    asset_revision: String,
    plan: MotionPlan,
}

enum ResultState {
    Pending(oneshot::Receiver<ExternalOutcome>),
    Ready(TaskArtifact),
    Failed,
}

struct Record {
    request: Value,
    task_id: u64,
    cancellation: CancellationToken,
    completed_at: Option<Instant>,
    discard: bool,
    state: ResultState,
}

impl Record {
    fn refresh(&mut self) {
        if let ResultState::Pending(receiver) = &mut self.state {
            match receiver.try_recv() {
                Ok(outcome) => {
                    self.completed_at = Some(outcome.completed_at);
                    self.state = match outcome.result {
                        Ok(artifact) => ResultState::Ready(artifact),
                        Err(error) => {
                            eprintln!("Remote motion failed: {error}");
                            ResultState::Failed
                        }
                    };
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.completed_at = Some(Instant::now());
                    self.state = ResultState::Failed;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
    }
}

pub struct MotionArtifact {
    pub data: Vec<u8>,
    pub file_name: String,
    pub content_type: String,
    pub duration_ms: Option<u32>,
}

pub(crate) struct RemoteMotionService {
    records: Mutex<HashMap<String, Record>>,
    tasks: Arc<TaskRuntime>,
    http: Arc<HttpService>,
    ffmpeg_path: Option<PathBuf>,
}

impl RemoteMotionService {
    pub(crate) fn new(
        tasks: Arc<TaskRuntime>,
        http: Arc<HttpService>,
        ffmpeg_path: Option<PathBuf>,
    ) -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
            tasks,
            http,
            ffmpeg_path,
        }
    }

    async fn prune(records: &mut HashMap<String, Record>) {
        for record in records.values_mut() {
            record.refresh();
        }
        let expired: Vec<String> = records
            .iter()
            .filter(|(_, record)| {
                record
                    .completed_at
                    .is_some_and(|at| record.discard || at.elapsed() >= RESULT_TTL)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(record) = records.remove(&id) {
                cleanup_task_workspace(record.task_id).await;
            }
        }
    }

    pub(crate) async fn submit(&self, value: Value) -> Result<(), &'static str> {
        let request: MotionRenderRequest =
            serde_json::from_value(value.clone()).map_err(|_| "invalid-request")?;
        validate(&request)?;
        let mut records = self.records.lock().await;
        Self::prune(&mut records).await;
        if let Some(existing) = records.get(&request.request_id) {
            return if existing.request == value && !existing.discard {
                Ok(())
            } else {
                Err("request-conflict")
            };
        }
        if records.len() >= RESULT_CAPACITY {
            return Err("busy");
        }
        let assets = RemoteAssetSource::new(
            format!(
                "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/{}/jp/sitedata",
                request.asset_revision
            ),
            RESULT_TTL,
            Arc::clone(&self.http),
        );
        let cancellation = CancellationToken::new();
        let (task_id, receiver) = self
            .tasks
            .submit_external(
                Box::new(MotionJob::new(
                    request.plan,
                    assets,
                    self.ffmpeg_path.clone(),
                )),
                cancellation.clone(),
            )
            .map_err(|error| match error {
                crate::task_runtime::TaskSubmitError::Busy => "busy",
                crate::task_runtime::TaskSubmitError::Unavailable => "unavailable",
            })?;
        records.insert(
            request.request_id,
            Record {
                request: value,
                task_id,
                cancellation,
                completed_at: None,
                discard: false,
                state: ResultState::Pending(receiver),
            },
        );
        Ok(())
    }

    pub(crate) async fn status(&self, id: &str) -> Option<Value> {
        let mut records = self.records.lock().await;
        Self::prune(&mut records).await;
        let record = records.get(id)?;
        Some(match &record.state {
            ResultState::Pending(_) => json!({"protocolVersion":1,"status":"pending"}),
            ResultState::Failed => json!({"protocolVersion":1,"status":"failed"}),
            ResultState::Ready(artifact) => json!({"protocolVersion":1,"status":"ready",
                "fileName":artifact.file_name,"contentType":artifact.content_type,"durationMs":artifact.duration_ms}),
        })
    }

    pub(crate) async fn read(&self, id: &str) -> Result<MotionArtifact, &'static str> {
        let mut records = self.records.lock().await;
        Self::prune(&mut records).await;
        let record = records.get(id).ok_or("not-found")?;
        let ResultState::Ready(artifact) = &record.state else {
            return Err("not-ready");
        };
        let metadata = tokio::fs::metadata(&artifact.path)
            .await
            .map_err(|_| "artifact-unavailable")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ATTACHMENT_BYTES {
            return Err("artifact-unavailable");
        }
        let data = tokio::fs::read(&artifact.path)
            .await
            .map_err(|_| "artifact-unavailable")?;
        if data.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err("artifact-unavailable");
        }
        Ok(MotionArtifact {
            data,
            file_name: artifact.file_name.clone(),
            content_type: artifact
                .content_type
                .clone()
                .ok_or("artifact-unavailable")?,
            duration_ms: artifact.duration_ms,
        })
    }

    pub(crate) async fn remove(&self, id: &str) {
        let mut records = self.records.lock().await;
        if let Some(record) = records.get_mut(id) {
            record.discard = true;
            record.cancellation.cancel();
        }
        Self::prune(&mut records).await;
    }

    pub(crate) async fn shutdown(&self) {
        let mut records = self.records.lock().await;
        for (_, record) in records.drain() {
            record.cancellation.cancel();
            cleanup_task_workspace(record.task_id).await;
        }
    }
}

fn validate(request: &MotionRenderRequest) -> Result<(), &'static str> {
    let plan = &request.plan;
    let valid_revision = request.asset_revision == "main"
        || (request.asset_revision.len() == 40
            && request
                .asset_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit()));
    let valid_id =
        request.request_id.len() == 40 && request.request_id.bytes().all(|b| b.is_ascii_hexdigit());
    let safe_name = !plan.filename_stem.is_empty()
        && plan.filename_stem.len() <= 64
        && plan
            .filename_stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b));
    let base = plan
        .sprite_path
        .strip_prefix("Number/")
        .and_then(|s| s.strip_suffix(".png"))
        .ok_or("invalid-request")?;
    if request.protocol_version != 1
        || !valid_revision
        || !valid_id
        || !safe_name
        || base.is_empty()
        || base.len() > 40
        || !base.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || !plan.preview_scale.is_finite()
        || !(0.0 < plan.preview_scale && plan.preview_scale <= 4.0)
        || plan.segments.is_empty()
        || plan.segments.len() > 32
        || plan.animation_paths.is_empty()
        || plan.animation_paths.len() > 4
        || plan.imgcut_path != format!("ImageData/{base}.imgcut")
        || plan.model_path != format!("ImageData/{base}.mamodel")
    {
        return Err("invalid-request");
    }
    let mut frames = 0u64;
    for segment in &plan.segments {
        if plan.animation_paths.get(&segment.motion)
            != Some(&format!(
                "ImageData/{base}0{}.maanim",
                segment.motion.asset_index()
            ))
        {
            return Err("invalid-request");
        }
        if let Some(range) = segment.range {
            if range.start > range.end {
                return Err("invalid-request");
            }
            frames += u64::from(range.end) - u64::from(range.start) + 1;
        }
    }
    if frames > 900
        || (plan.format == MotionFormat::Png
            && (plan.segments.len() != 1
                || plan.segments[0].range.is_none_or(|r| r.start != r.end)))
    {
        return Err("invalid-request");
    }
    for (kind, path) in &plan.animation_paths {
        if path != &format!("ImageData/{base}0{}.maanim", kind.asset_index()) {
            return Err("invalid-request");
        }
    }
    Ok(())
}
