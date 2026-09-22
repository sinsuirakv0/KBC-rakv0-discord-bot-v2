//! Rust CoreをNode.jsへ公開する薄いN-API Bridge。

use kbc_core::{AppRuntime, RuntimeConfig, RuntimeError, StorageConfig};
use kbc_protocol::{CoreEvent, PROTOCOL_VERSION, RuntimeInfo};
use napi::{Error, Result, Status};
use napi_derive::napi;
use std::time::Duration;

#[napi(object)]
pub struct NativeRuntimeConfig {
    pub event_queue_capacity: Option<u32>,
    pub action_queue_capacity: Option<u32>,
    pub content_directory: Option<String>,
    pub ffmpeg_path: Option<String>,
    pub http_max_concurrency: Option<u32>,
    pub http_request_timeout_ms: Option<u32>,
    pub http_max_response_bytes: Option<u32>,
    pub github_data_owner: Option<String>,
    pub github_data_repository: Option<String>,
    pub github_data_branch: Option<String>,
    pub github_data_token: Option<String>,
}

#[napi]
pub struct NativeCore {
    runtime: AppRuntime,
}

#[napi]
impl NativeCore {
    #[napi(js_name = "submitEvent")]
    pub async fn submit_event(&self, value: serde_json::Value) -> Result<()> {
        let event: CoreEvent = serde_json::from_value(value)
            .map_err(|error| protocol_input_error(error.to_string()))?;
        self.runtime
            .submit_event(event)
            .await
            .map_err(runtime_error)
    }

    #[napi(js_name = "nextAction")]
    pub async fn next_action(&self) -> Result<Option<serde_json::Value>> {
        self.runtime
            .next_action()
            .await
            .map_err(runtime_error)?
            .map(serde_json::to_value)
            .transpose()
            .map_err(bridge_error)
    }

    #[napi]
    pub async fn shutdown(&self) -> Result<()> {
        self.runtime.shutdown().await.map_err(runtime_error)
    }

    #[napi(js_name = "prepareNotifications")]
    pub async fn prepare_notifications(&self) -> Result<()> {
        self.runtime
            .prepare_notifications()
            .await
            .map_err(notification_error)
    }

    #[napi(js_name = "submitDetection")]
    pub async fn submit_detection(&self, value: serde_json::Value) -> Result<()> {
        self.runtime
            .submit_detection(value)
            .await
            .map_err(notification_error)
    }
}

#[napi(js_name = "createCore")]
pub async fn create_core(config: Option<NativeRuntimeConfig>) -> Result<NativeCore> {
    let mut runtime_config = RuntimeConfig::default();
    if let Some(config) = config {
        if let Some(capacity) = config.event_queue_capacity {
            runtime_config.event_queue_capacity = capacity as usize;
        }
        if let Some(capacity) = config.action_queue_capacity {
            runtime_config.action_queue_capacity = capacity as usize;
        }
        if let Some(directory) = config.content_directory {
            runtime_config.content_directory = directory.into();
        }
        if let Some(path) = config.ffmpeg_path {
            runtime_config.ffmpeg_path = Some(path.into());
        }
        if let Some(max_concurrency) = config.http_max_concurrency {
            runtime_config.http_max_concurrency = max_concurrency as usize;
        }
        if let Some(timeout_ms) = config.http_request_timeout_ms {
            runtime_config.http_request_timeout = Duration::from_millis(timeout_ms as u64);
        }
        if let Some(max_response_bytes) = config.http_max_response_bytes {
            runtime_config.http_max_response_bytes = max_response_bytes as usize;
        }
        if let (Some(owner), Some(repository), Some(branch), Some(token)) = (
            config.github_data_owner,
            config.github_data_repository,
            config.github_data_branch,
            config.github_data_token,
        ) {
            runtime_config.storage = Some(StorageConfig {
                owner,
                repository,
                branch,
                token,
            });
        }
    }
    let runtime = AppRuntime::start(runtime_config).map_err(runtime_error)?;

    Ok(NativeCore { runtime })
}

#[napi(js_name = "getRuntimeInfo")]
pub fn get_runtime_info() -> Result<serde_json::Value> {
    let info = RuntimeInfo {
        protocol_version: PROTOCOL_VERSION,
        core_version: env!("CARGO_PKG_VERSION").to_owned(),
    };

    serde_json::to_value(info).map_err(bridge_error)
}

fn protocol_input_error(reason: String) -> Error {
    Error::new(Status::InvalidArg, format!("invalid CoreEvent: {reason}"))
}

fn runtime_error(error: RuntimeError) -> Error {
    let status = match &error {
        RuntimeError::InvalidQueueCapacity { .. }
        | RuntimeError::InvalidConfiguration { .. }
        | RuntimeError::ProtocolVersion(_) => Status::InvalidArg,
        _ => Status::GenericFailure,
    };
    Error::new(status, error.to_string())
}

fn bridge_error(error: serde_json::Error) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

fn notification_error(error: kbc_core::NotificationError) -> Error {
    Error::new(Status::GenericFailure, error.code())
}
