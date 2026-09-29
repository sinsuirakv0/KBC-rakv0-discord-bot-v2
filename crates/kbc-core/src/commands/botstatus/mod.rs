//! Northflankの稼働情報をEmbedと自動更新Buttonで表示する。

mod data_source;
mod graph;
mod model;

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use kbc_protocol::{
    ActionOutcome, ButtonStyle, CoreActionData, CoreEvent, CoreEventData, MessageAttachment,
    MessageComponentData, MessageComponentRow, MessageEmbed, MessageEmbedField, RequestId,
    RichMessage,
};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval, interval_at};

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::runtime::BotStatusConfig;
use crate::services::{Clock, HttpService};
use crate::storage::{BotUptimeRecord, StorageService, merge_bot_uptime};
use crate::task_runtime::TaskRuntime;

use self::data_source::{BotStatusDataError, NorthflankDataSource};
use self::model::{MetricSeries, StatusRange, StatusSnapshot};

const STATUS_PREFIX: &str = "botstatus:";
const INITIAL_V2_COMMIT_SHA: &str = "976c7c351a6bc905af082128b6b6d18fd47f3366";
const INITIAL_V2_FALLBACK_AT: &str = "2026-09-22T03:07:19Z";
const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SESSION_RETENTION: Duration = Duration::from_secs(15 * 60);
const UPTIME_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(60 * 60);
const MAX_SESSIONS: usize = 8;
const GRAPH_FILE_NAME: &str = "botstatus.png";

pub(super) struct BotStatusCommand {
    metadata: CommandMetadata,
    help: String,
    service: Arc<BotStatusService>,
}

impl BotStatusCommand {
    pub(super) fn new(help: &str, service: Arc<BotStatusService>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "botstatus".to_owned(),
                aliases: Vec::new(),
                guild_only: false,
            },
            help: help.to_owned(),
            service,
        }
    }

    async fn run(&self, context: &CommandContext, arguments: &[String]) -> CommandOutput {
        if arguments.len() == 1 && arguments[0].eq_ignore_ascii_case("help") {
            return message(context.channel_id(), &self.help);
        }
        if !arguments.is_empty() {
            return message(context.channel_id(), "使い方: o.botstatus");
        }
        if !self.service.is_configured() {
            return message(
                context.channel_id(),
                "❌ Bot statusは現在の実行環境で設定されていません。",
            );
        }
        match self.service.open(context).await {
            Ok(()) => CommandOutput::new(),
            Err(BotStatusError::Busy) => message(
                context.channel_id(),
                "現在開いているBot status画面が多すぎます。しばらく待ってから再実行してください。",
            ),
            Err(error) => {
                eprintln!("Bot status command failed: {error}");
                message(
                    context.channel_id(),
                    "❌ Botの稼働情報を取得できませんでした。時間をおいて再度お試しください。",
                )
            }
        }
    }
}

impl Command for BotStatusCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { Ok(self.run(&context, &arguments).await) })
    }
}

struct StatusSession {
    owner_id: String,
    channel_id: String,
    message_id: String,
    request_id: RequestId,
    range: StatusRange,
    running: bool,
    idle_deadline: Instant,
    retention_deadline: Instant,
    next_refresh: Instant,
    last_message: RichMessage,
}

#[derive(Default)]
struct UptimeState {
    record: Option<BotUptimeRecord>,
    loaded: bool,
    dirty: bool,
    estimated_baseline: bool,
    persistence_available: bool,
}

#[derive(Clone, Copy)]
struct UptimeValue {
    seconds: u64,
    estimated: bool,
    persistent: bool,
}

pub(crate) struct BotStatusService {
    data_source: Option<Arc<NorthflankDataSource>>,
    storage: Arc<StorageService>,
    tasks: Arc<TaskRuntime>,
    clock: Arc<dyn Clock>,
    process_started_at: DateTime<Utc>,
    sessions: Mutex<HashMap<String, StatusSession>>,
    refresh_lock: Mutex<()>,
    uptime: Mutex<UptimeState>,
    uptime_write_lock: Mutex<()>,
    shutdown_receiver: watch::Receiver<bool>,
    worker: StdMutex<Option<JoinHandle<()>>>,
}

impl BotStatusService {
    pub(crate) fn start(
        config: Option<BotStatusConfig>,
        http: Arc<HttpService>,
        storage: Arc<StorageService>,
        tasks: Arc<TaskRuntime>,
        clock: Arc<dyn Clock>,
        shutdown_receiver: watch::Receiver<bool>,
    ) -> Arc<Self> {
        let process_started_at = clock.now();
        let data_source = config.and_then(|config| {
            NorthflankDataSource::new(config, http)
                .map(Arc::new)
                .map_err(|error| eprintln!("Bot status configuration is invalid: {error}"))
                .ok()
        });
        let service = Arc::new(Self {
            data_source,
            storage,
            tasks,
            clock,
            process_started_at,
            sessions: Mutex::new(HashMap::new()),
            refresh_lock: Mutex::new(()),
            uptime: Mutex::new(UptimeState::default()),
            uptime_write_lock: Mutex::new(()),
            shutdown_receiver,
            worker: StdMutex::new(None),
        });
        let worker = tokio::spawn(Arc::clone(&service).run_worker());
        *service.worker.lock().expect("bot status worker lock") = Some(worker);
        service
    }

    pub(crate) fn is_configured(&self) -> bool {
        self.data_source.is_some()
    }

    pub(crate) fn handles_event(event: &CoreEvent) -> bool {
        matches!(
            &event.event,
            CoreEventData::ComponentInteraction { custom_id, .. }
                if custom_id.starts_with(STATUS_PREFIX)
        )
    }

    async fn open(&self, context: &CommandContext) -> Result<(), BotStatusError> {
        if self.sessions.lock().await.len() >= MAX_SESSIONS {
            return Err(BotStatusError::Busy);
        }
        let _refresh = self.refresh_lock.lock().await;
        let range = StatusRange::OneHour;
        let message = self
            .status_message(context.user_id(), range, true, None)
            .await?;
        let outcome = self
            .tasks
            .request_adapter(
                context.request_id(),
                CoreActionData::SendRichMessage {
                    channel_id: context.channel_id().to_owned(),
                    message: message.clone(),
                },
            )
            .await
            .map_err(BotStatusError::Adapter)?;
        let ActionOutcome::Success {
            message_id: Some(message_id),
        } = outcome
        else {
            return Err(BotStatusError::Adapter("status message was not created"));
        };
        let now = Instant::now();
        self.sessions.lock().await.insert(
            message_id.clone(),
            StatusSession {
                owner_id: context.user_id().to_owned(),
                channel_id: context.channel_id().to_owned(),
                message_id,
                request_id: context.request_id().clone(),
                range,
                running: true,
                idle_deadline: now + IDLE_TIMEOUT,
                retention_deadline: now + SESSION_RETENTION,
                next_refresh: now + AUTO_REFRESH_INTERVAL,
                last_message: message,
            },
        );
        self.persist_uptime().await;
        Ok(())
    }

    pub(crate) async fn handle_event(&self, event: &CoreEvent) {
        let CoreEventData::ComponentInteraction {
            interaction_id,
            channel_id,
            message_id,
            user_id,
            custom_id,
            ..
        } = &event.event
        else {
            return;
        };
        let Some((owner_id, action)) = parse_action(custom_id) else {
            self.expired_interaction(interaction_id).await;
            return;
        };
        let session_matches = self
            .sessions
            .lock()
            .await
            .get(message_id)
            .is_some_and(|session| {
                session.owner_id == *user_id
                    && session.owner_id == owner_id
                    && session.channel_id == *channel_id
            });
        if !session_matches {
            self.expired_interaction(interaction_id).await;
            return;
        }

        match action {
            StatusAction::Stop => {
                let message = {
                    let mut sessions = self.sessions.lock().await;
                    let Some(session) = sessions.get_mut(message_id) else {
                        return;
                    };
                    let now = Instant::now();
                    session.running = false;
                    session.retention_deadline = now + SESSION_RETENTION;
                    set_activity(
                        &mut session.last_message,
                        &session.owner_id,
                        session.range,
                        ActivityState::Stopped,
                    );
                    session.last_message.clone_without_attachments()
                };
                self.edit_interaction(interaction_id, message).await;
                self.advance_uptime(self.clock.now()).await;
                self.persist_uptime().await;
            }
            StatusAction::Refresh | StatusAction::Resume | StatusAction::Range(_) => {
                let _refresh = self.refresh_lock.lock().await;
                let (owner_id, range) = {
                    let mut sessions = self.sessions.lock().await;
                    let Some(session) = sessions.get_mut(message_id) else {
                        return;
                    };
                    if let StatusAction::Range(range) = action {
                        session.range = range;
                    }
                    let now = Instant::now();
                    session.running = true;
                    session.idle_deadline = now + IDLE_TIMEOUT;
                    session.retention_deadline = now + SESSION_RETENTION;
                    session.next_refresh = now + AUTO_REFRESH_INTERVAL;
                    (session.owner_id.clone(), session.range)
                };
                match self.status_message(&owner_id, range, true, None).await {
                    Ok(message) => {
                        self.edit_interaction(interaction_id, message.clone()).await;
                        if let Some(session) = self.sessions.lock().await.get_mut(message_id) {
                            session.last_message = message;
                        }
                    }
                    Err(error) => {
                        eprintln!("Bot status interaction refresh failed: {error}");
                        let message = self.session_error_message(message_id).await;
                        self.edit_interaction(interaction_id, message).await;
                    }
                }
            }
        }
    }

    pub(crate) async fn shutdown(&self) {
        let worker = self.worker.lock().expect("bot status worker lock").take();
        if let Some(worker) = worker
            && let Err(error) = worker.await
        {
            eprintln!("Bot status worker failed during shutdown: {error}");
        }
        self.persist_uptime().await;
    }

    async fn run_worker(self: Arc<Self>) {
        self.ensure_uptime(self.process_started_at).await;
        self.persist_uptime().await;
        let mut tick = interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut uptime_checkpoint = interval_at(
            Instant::now() + UPTIME_CHECKPOINT_INTERVAL,
            UPTIME_CHECKPOINT_INTERVAL,
        );
        uptime_checkpoint.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut shutdown = self.shutdown_receiver.clone();
        loop {
            tokio::select! {
                biased;
                _ = wait_for_shutdown(&mut shutdown) => break,
                _ = tick.tick() => self.tick().await,
                _ = uptime_checkpoint.tick() => {
                    self.advance_uptime(self.clock.now()).await;
                    self.persist_uptime().await;
                },
            }
        }
        self.advance_uptime(self.clock.now()).await;
        self.persist_uptime().await;
    }

    async fn tick(&self) {
        enum Work {
            Refresh {
                message_id: String,
                owner_id: String,
                range: StatusRange,
                channel_id: String,
                request_id: RequestId,
            },
            Edit {
                channel_id: String,
                message_id: String,
                request_id: RequestId,
                message: RichMessage,
            },
        }

        let now = Instant::now();
        let mut work = Vec::new();
        let mut should_persist = false;
        {
            let mut sessions = self.sessions.lock().await;
            let expired = sessions
                .iter()
                .filter(|(_, session)| !session.running && now >= session.retention_deadline)
                .map(|(message_id, _)| message_id.clone())
                .collect::<Vec<_>>();
            for message_id in expired {
                if let Some(mut session) = sessions.remove(&message_id) {
                    set_activity(
                        &mut session.last_message,
                        &session.owner_id,
                        session.range,
                        ActivityState::Expired,
                    );
                    work.push(Work::Edit {
                        channel_id: session.channel_id,
                        message_id: session.message_id,
                        request_id: session.request_id,
                        message: session.last_message.clone_without_attachments(),
                    });
                }
            }
            for session in sessions.values_mut() {
                if session.running && now >= session.idle_deadline {
                    session.running = false;
                    session.retention_deadline = now + SESSION_RETENTION;
                    set_activity(
                        &mut session.last_message,
                        &session.owner_id,
                        session.range,
                        ActivityState::Stopped,
                    );
                    work.push(Work::Edit {
                        channel_id: session.channel_id.clone(),
                        message_id: session.message_id.clone(),
                        request_id: session.request_id.clone(),
                        message: session.last_message.clone_without_attachments(),
                    });
                    should_persist = true;
                } else if session.running && now >= session.next_refresh {
                    session.next_refresh = now + AUTO_REFRESH_INTERVAL;
                    work.push(Work::Refresh {
                        message_id: session.message_id.clone(),
                        owner_id: session.owner_id.clone(),
                        range: session.range,
                        channel_id: session.channel_id.clone(),
                        request_id: session.request_id.clone(),
                    });
                }
            }
        }

        for item in work {
            match item {
                Work::Refresh {
                    message_id,
                    owner_id,
                    range,
                    channel_id,
                    request_id,
                } => {
                    let _refresh = self.refresh_lock.lock().await;
                    if !self
                        .sessions
                        .lock()
                        .await
                        .get(&message_id)
                        .is_some_and(|session| session.running && session.range == range)
                    {
                        continue;
                    }
                    match self.status_message(&owner_id, range, true, None).await {
                        Ok(message) => {
                            if self
                                .edit_message(
                                    &request_id,
                                    &channel_id,
                                    &message_id,
                                    message.clone(),
                                )
                                .await
                            {
                                if let Some(session) =
                                    self.sessions.lock().await.get_mut(&message_id)
                                {
                                    session.last_message = message;
                                }
                            }
                        }
                        Err(error) => eprintln!("Bot status auto refresh failed: {error}"),
                    }
                }
                Work::Edit {
                    channel_id,
                    message_id,
                    request_id,
                    message,
                } => {
                    self.edit_message(&request_id, &channel_id, &message_id, message)
                        .await;
                }
            }
        }
        if should_persist {
            self.advance_uptime(self.clock.now()).await;
            self.persist_uptime().await;
        }
    }

    async fn status_message(
        &self,
        owner_id: &str,
        range: StatusRange,
        running: bool,
        note: Option<&str>,
    ) -> Result<RichMessage, BotStatusError> {
        let data_source = self
            .data_source
            .as_ref()
            .ok_or(BotStatusError::NotConfigured)?;
        let now = self.clock.now();
        let snapshot = data_source.snapshot(range, now).await?;
        let uptime = self.update_uptime(snapshot.fetched_at).await;
        let graph = match graph::render_graph(&snapshot.cpu, &snapshot.memory) {
            Ok(graph) => Some(graph),
            Err(error) => {
                eprintln!("Bot status graph generation failed: {error}");
                None
            }
        };
        Ok(format_message(
            data_source.config(),
            owner_id,
            range,
            running,
            &snapshot,
            uptime,
            graph,
            note,
        ))
    }

    async fn update_uptime(&self, now: DateTime<Utc>) -> UptimeValue {
        self.ensure_uptime(now).await;
        self.advance_uptime(now).await;
        let uptime = self.uptime.lock().await;
        let Some(record) = uptime.record.as_ref() else {
            return UptimeValue {
                seconds: 0,
                estimated: true,
                persistent: false,
            };
        };
        UptimeValue {
            seconds: record.accumulated_seconds,
            estimated: uptime.estimated_baseline,
            persistent: uptime.persistence_available,
        }
    }

    async fn advance_uptime(&self, now: DateTime<Utc>) {
        let mut uptime = self.uptime.lock().await;
        let Some(record) = uptime.record.as_mut() else {
            return;
        };
        let accounted = parse_utc(&record.accounted_through).unwrap_or(now);
        if now > accounted {
            let start = std::cmp::max(accounted, self.process_started_at);
            let additional_seconds = now.signed_duration_since(start).num_seconds().max(0) as u64;
            record.accumulated_seconds = record
                .accumulated_seconds
                .saturating_add(additional_seconds);
            record.accounted_through = timestamp(now);
            uptime.dirty = true;
        }
    }

    async fn ensure_uptime(&self, now: DateTime<Utc>) {
        if self.uptime.lock().await.loaded {
            return;
        }
        let persistence_available = self.storage.is_configured();
        let stored = if persistence_available {
            match self.storage.bot_uptime().await {
                Ok(record) => record,
                Err(error) => {
                    eprintln!("Bot uptime load failed: {error}");
                    None
                }
            }
        } else {
            None
        };
        let (record, estimated_baseline, dirty) = if let Some(record) = stored {
            let estimated = record.baseline_estimated;
            (Some(record), estimated, false)
        } else {
            let (baseline, estimated) = self.initial_baseline().await;
            let seconds = now.signed_duration_since(baseline).num_seconds().max(0) as u64;
            (
                Some(BotUptimeRecord {
                    schema_version: 1,
                    baseline_at: timestamp(baseline),
                    accounted_through: timestamp(now),
                    accumulated_seconds: seconds,
                    baseline_estimated: estimated,
                }),
                estimated,
                true,
            )
        };
        let mut uptime = self.uptime.lock().await;
        if !uptime.loaded {
            uptime.record = record;
            uptime.loaded = true;
            uptime.dirty = dirty;
            uptime.estimated_baseline = estimated_baseline;
            uptime.persistence_available = persistence_available;
        }
    }

    async fn initial_baseline(&self) -> (DateTime<Utc>, bool) {
        let Some(data_source) = &self.data_source else {
            return (
                parse_utc(INITIAL_V2_FALLBACK_AT).expect("valid fallback"),
                true,
            );
        };
        if let Some(configured) = data_source
            .config()
            .uptime_started_at
            .as_deref()
            .and_then(parse_utc)
        {
            return (configured, false);
        }
        match data_source
            .initial_v2_deployment_at(INITIAL_V2_COMMIT_SHA)
            .await
        {
            Ok(Some(value)) => (value, false),
            Ok(None) => (
                parse_utc(INITIAL_V2_FALLBACK_AT).expect("valid fallback"),
                true,
            ),
            Err(error) => {
                eprintln!("Initial V2 deployment lookup failed: {error}");
                (
                    parse_utc(INITIAL_V2_FALLBACK_AT).expect("valid fallback"),
                    true,
                )
            }
        }
    }

    async fn persist_uptime(&self) {
        if !self.storage.is_configured() {
            return;
        }
        let _write = self.uptime_write_lock.lock().await;
        let record = {
            let uptime = self.uptime.lock().await;
            if !uptime.dirty {
                return;
            }
            uptime.record.clone()
        };
        let Some(record) = record else {
            return;
        };
        match self
            .storage
            .set_bot_uptime(&record, &timestamp(self.process_started_at))
            .await
        {
            Ok(saved) => {
                let mut uptime = self.uptime.lock().await;
                let merged = uptime
                    .record
                    .as_ref()
                    .and_then(|current| {
                        merge_bot_uptime(
                            Some(&saved),
                            current,
                            self.process_started_at.fixed_offset(),
                        )
                        .ok()
                    })
                    .unwrap_or_else(|| saved.clone());
                uptime.dirty = merged.accounted_through != saved.accounted_through;
                uptime.estimated_baseline = merged.baseline_estimated;
                uptime.record = Some(merged);
            }
            Err(error) => eprintln!("Bot uptime save failed: {error}"),
        }
    }

    async fn edit_interaction(&self, interaction_id: &str, message: RichMessage) {
        if let Err(error) = self
            .tasks
            .request_adapter(
                &RequestId::new(format!("request:botstatus:{interaction_id}")),
                CoreActionData::EditRichInteractionReply {
                    interaction_id: interaction_id.to_owned(),
                    message,
                },
            )
            .await
        {
            eprintln!("Bot status interaction reply failed: {error}");
        }
    }

    async fn edit_message(
        &self,
        request_id: &RequestId,
        channel_id: &str,
        message_id: &str,
        message: RichMessage,
    ) -> bool {
        matches!(
            self.tasks
                .request_adapter(
                    request_id,
                    CoreActionData::EditRichMessage {
                        channel_id: channel_id.to_owned(),
                        message_id: message_id.to_owned(),
                        message,
                    },
                )
                .await,
            Ok(ActionOutcome::Success { .. })
        )
    }

    async fn expired_interaction(&self, interaction_id: &str) {
        self.edit_interaction(
            interaction_id,
            RichMessage {
                content: Some("このBot status画面の操作受付は終了しました。".to_owned()),
                embeds: Vec::new(),
                rows: Vec::new(),
                attachments: Vec::new(),
            },
        )
        .await;
    }

    async fn session_error_message(&self, message_id: &str) -> RichMessage {
        let mut message = self
            .sessions
            .lock()
            .await
            .get(message_id)
            .map(|session| session.last_message.clone_without_attachments())
            .unwrap_or_else(|| RichMessage {
                content: None,
                embeds: Vec::new(),
                rows: Vec::new(),
                attachments: Vec::new(),
            });
        if let Some(embed) = message.embeds.first_mut() {
            embed.footer = Some("更新に失敗しました。もう一度お試しください。".to_owned());
        }
        message
    }
}

trait RichMessageExt {
    fn clone_without_attachments(&self) -> Self;
}

impl RichMessageExt for RichMessage {
    fn clone_without_attachments(&self) -> Self {
        let mut message = self.clone();
        message.attachments.clear();
        message
    }
}

#[derive(Clone, Copy)]
enum StatusAction {
    Range(StatusRange),
    Refresh,
    Stop,
    Resume,
}

#[derive(Clone, Copy)]
enum ActivityState {
    Running,
    Stopped,
    Expired,
}

fn parse_action(custom_id: &str) -> Option<(String, StatusAction)> {
    let mut parts = custom_id.split(':');
    (parts.next()? == "botstatus").then_some(())?;
    let owner_id = parts.next()?.to_owned();
    let action = match (parts.next()?, parts.next()) {
        ("range", Some("30m")) => StatusAction::Range(StatusRange::ThirtyMinutes),
        ("range", Some("1h")) => StatusAction::Range(StatusRange::OneHour),
        ("refresh", None) => StatusAction::Refresh,
        ("stop", None) => StatusAction::Stop,
        ("resume", None) => StatusAction::Resume,
        _ => return None,
    };
    Some((owner_id, action))
}

fn format_message(
    config: &BotStatusConfig,
    owner_id: &str,
    range: StatusRange,
    running: bool,
    snapshot: &StatusSnapshot,
    uptime: UptimeValue,
    graph: Option<Vec<u8>>,
    note: Option<&str>,
) -> RichMessage {
    let running_containers = snapshot
        .containers
        .iter()
        .filter(|container| container.is_running())
        .count();
    let healthy = snapshot.service.deployment_status.as_deref() == Some("COMPLETED")
        && running_containers > 0;
    let status = if healthy {
        "🟢 正常稼働中"
    } else if running_containers > 0 {
        "🟡 状態を確認中"
    } else {
        "🔴 停止または異常"
    };
    let current_uptime = current_uptime(config, snapshot)
        .map(format_duration)
        .unwrap_or_else(|| "取得不可".to_owned());
    let mut total_uptime = format_duration(uptime.seconds);
    if uptime.estimated {
        total_uptime.push_str("（開始時刻は推定）");
    }
    if !uptime.persistent {
        total_uptime.push_str("（保存無効）");
    }
    let container_text = format!(
        "{} / {} Running",
        running_containers,
        snapshot
            .service
            .instances
            .unwrap_or(snapshot.containers.len() as u32)
    );
    let cpu = metric_summary(&snapshot.cpu);
    let memory = metric_summary(&snapshot.memory);
    let network = format!(
        "受信 {}\n送信 {}\nRequests {}\n5xx {}",
        metric_latest(&snapshot.network_ingress),
        metric_latest(&snapshot.network_egress),
        metric_latest(&snapshot.requests),
        metric_latest(&snapshot.http_5xx),
    );
    let environment = format!(
        "Northflank {} / {}\nPlan: {} / Instances: {}\n{} / {}",
        snapshot
            .service
            .service_type
            .as_deref()
            .unwrap_or("service"),
        snapshot
            .service
            .region
            .as_deref()
            .unwrap_or("unknown region"),
        snapshot
            .service
            .deployment_plan
            .as_deref()
            .unwrap_or("unknown"),
        snapshot
            .service
            .instances
            .unwrap_or(running_containers as u32),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    let commit = snapshot
        .service
        .commit_sha
        .as_deref()
        .map(|sha| sha.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "unknown".to_owned());
    let deployment = format!(
        "{} / `{}`\nCore {} / Node {}\nDeploy: {} / Build: {}",
        snapshot.service.branch.as_deref().unwrap_or("unknown"),
        commit,
        env!("CARGO_PKG_VERSION"),
        config.node_version.as_deref().unwrap_or("unknown"),
        snapshot
            .service
            .deployment_status
            .as_deref()
            .unwrap_or("unknown"),
        snapshot
            .service
            .build_status
            .as_deref()
            .unwrap_or("unknown"),
    );
    let description = note
        .map(|note| format!("{status}\n{note}"))
        .unwrap_or_else(|| status.to_owned());
    let footer = activity_footer(
        range,
        if running {
            ActivityState::Running
        } else {
            ActivityState::Stopped
        },
        snapshot.api_elapsed,
    );
    let has_graph = graph.is_some();
    let embed = MessageEmbed {
        title: Some(format!(
            "{} Bot Status",
            snapshot.service.name.as_deref().unwrap_or("KBC")
        )),
        description: Some(description),
        color: Some(if healthy { 0x3f_b9_50 } else { 0xd2_99_22 }),
        fields: vec![
            field("稼働時間", current_uptime, true),
            field("累計稼働時間", total_uptime, true),
            field("Container", container_text, true),
            field("🟦 CPU", cpu, true),
            field("🟪 Memory", memory, true),
            field("Network", network, true),
            field("実行環境", environment, false),
            field("デプロイ", deployment, false),
        ],
        footer: Some(footer),
        timestamp: Some(timestamp(snapshot.fetched_at)),
        image_attachment: has_graph.then(|| GRAPH_FILE_NAME.to_owned()),
    };
    RichMessage {
        content: None,
        embeds: vec![embed],
        rows: status_rows(owner_id, range, running, false),
        attachments: graph
            .map(|data| {
                vec![MessageAttachment {
                    file_name: GRAPH_FILE_NAME.to_owned(),
                    content_type: Some("image/png".to_owned()),
                    data,
                }]
            })
            .unwrap_or_default(),
    }
}

fn set_activity(
    message: &mut RichMessage,
    owner_id: &str,
    range: StatusRange,
    state: ActivityState,
) {
    let running = matches!(state, ActivityState::Running);
    let expired = matches!(state, ActivityState::Expired);
    message.rows = status_rows(owner_id, range, running, expired);
    if let Some(embed) = message.embeds.first_mut() {
        embed.footer = Some(activity_footer(range, state, Duration::ZERO));
    }
}

fn activity_footer(range: StatusRange, state: ActivityState, api_elapsed: Duration) -> String {
    let state = match state {
        ActivityState::Running => "15秒ごとに自動更新・最後の操作から1分で停止",
        ActivityState::Stopped => "自動更新停止中・ボタンで再開できます",
        ActivityState::Expired => "操作受付終了・再度 o.botstatus を実行してください",
    };
    if api_elapsed.is_zero() {
        format!("{}・{state}", range.label())
    } else {
        format!(
            "{}・{state}・Northflank {} ms",
            range.label(),
            api_elapsed.as_millis()
        )
    }
}

fn status_rows(
    owner_id: &str,
    range: StatusRange,
    running: bool,
    expired: bool,
) -> Vec<MessageComponentRow> {
    vec![MessageComponentRow {
        components: vec![
            button(
                format!("botstatus:{owner_id}:range:30m"),
                "30分",
                if range == StatusRange::ThirtyMinutes {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
                expired,
            ),
            button(
                format!("botstatus:{owner_id}:range:1h"),
                "1時間",
                if range == StatusRange::OneHour {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
                expired,
            ),
            button(
                format!(
                    "botstatus:{owner_id}:{}",
                    if running { "refresh" } else { "resume" }
                ),
                if running {
                    "今すぐ更新"
                } else {
                    "更新を再開"
                },
                ButtonStyle::Success,
                expired,
            ),
            button(
                format!("botstatus:{owner_id}:stop"),
                "停止",
                ButtonStyle::Danger,
                expired || !running,
            ),
        ],
    }]
}

fn button(
    custom_id: String,
    label: &str,
    style: ButtonStyle,
    disabled: bool,
) -> MessageComponentData {
    MessageComponentData::Button {
        custom_id,
        label: label.to_owned(),
        style,
        disabled,
    }
}

fn field(name: &str, value: String, inline: bool) -> MessageEmbedField {
    MessageEmbedField {
        name: name.to_owned(),
        value,
        inline,
    }
}

fn metric_summary(series: &MetricSeries) -> String {
    match (series.latest(), series.average(), series.maximum()) {
        (Some(latest), Some(average), Some(maximum)) => format!(
            "現在 {}\n平均 {}\n最大 {}",
            format_metric(latest, &series.unit),
            format_metric(average, &series.unit),
            format_metric(maximum, &series.unit),
        ),
        _ => "データなし".to_owned(),
    }
}

fn metric_latest(series: &MetricSeries) -> String {
    series
        .latest()
        .map(|value| format_metric(value, &series.unit))
        .unwrap_or_else(|| "--".to_owned())
}

fn format_metric(value: f64, unit: &str) -> String {
    match unit {
        "pct" => format!("{value:.1}%"),
        "vCPU" => format!("{value:.3} vCPU"),
        "mb" => format!("{value:.0} MB"),
        "kbps" => format!("{value:.1} kbps"),
        "rps" => format!("{value:.2} req/s"),
        "count" => format!("{value:.0}"),
        _ if unit.is_empty() => format!("{value:.2}"),
        _ => format!("{value:.2} {unit}"),
    }
}

fn current_uptime(config: &BotStatusConfig, snapshot: &StatusSnapshot) -> Option<u64> {
    let container = config
        .container_name
        .as_deref()
        .and_then(|name| {
            snapshot
                .containers
                .iter()
                .find(|container| container.is_running() && container.name == name)
        })
        .or_else(|| {
            snapshot
                .containers
                .iter()
                .filter(|container| container.is_running())
                .max_by_key(|container| container.created_at)
        })?;
    Some(
        snapshot
            .fetched_at
            .signed_duration_since(container.created_at)
            .num_seconds()
            .max(0) as u64,
    )
}

fn format_duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    if days > 0 {
        format!("{days}日 {hours}時間 {minutes}分")
    } else if hours > 0 {
        format!("{hours}時間 {minutes}分")
    } else {
        format!("{minutes}分")
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn message(channel_id: &str, content: &str) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.to_owned(),
    })
}

async fn wait_for_shutdown(receiver: &mut watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
}

#[derive(Debug)]
enum BotStatusError {
    Busy,
    NotConfigured,
    Data(BotStatusDataError),
    Adapter(&'static str),
}

impl Display for BotStatusError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => formatter.write_str("bot status session limit reached"),
            Self::NotConfigured => formatter.write_str("bot status is not configured"),
            Self::Data(error) => Display::fmt(error, formatter),
            Self::Adapter(reason) => write!(formatter, "Discord adapter action failed: {reason}"),
        }
    }
}

impl From<BotStatusDataError> for BotStatusError {
    fn from(error: BotStatusDataError) -> Self {
        Self::Data(error)
    }
}
