use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kbc_protocol::{CoreAction, CoreEvent, CoreEventData, EventId, ProtocolVersionMismatch};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::action_bus::ActionBus;
use crate::command_runtime::{CommandActionBatch, CommandRuntime};
use crate::content::ContentCatalog;
use crate::notification::{NotificationError, NotificationService};
use crate::role_panel::RolePanelService;
use crate::services::{HttpService, HttpServiceConfig, SystemClock};
use crate::session::{DEFAULT_MAX_SESSIONS, SessionManager, SessionRegistration};
use crate::storage::{StorageConfig, StorageService};
use crate::task_runtime::TaskRuntime;

pub const DEFAULT_EVENT_QUEUE_CAPACITY: usize = 64;
pub const DEFAULT_ACTION_QUEUE_CAPACITY: usize = 16;
pub const MAX_QUEUE_CAPACITY: usize = 1024;
pub const DEFAULT_HTTP_MAX_CONCURRENCY: usize = 4;
pub const DEFAULT_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_HTTP_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HTTP_CONCURRENCY: usize = 64;
const MAX_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_HTTP_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub event_queue_capacity: usize,
    pub action_queue_capacity: usize,
    pub content_directory: PathBuf,
    pub ffmpeg_path: Option<PathBuf>,
    pub http_max_concurrency: usize,
    pub http_request_timeout: Duration,
    pub http_max_response_bytes: usize,
    pub storage: Option<StorageConfig>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            event_queue_capacity: DEFAULT_EVENT_QUEUE_CAPACITY,
            action_queue_capacity: DEFAULT_ACTION_QUEUE_CAPACITY,
            content_directory: PathBuf::from("content"),
            ffmpeg_path: None,
            http_max_concurrency: DEFAULT_HTTP_MAX_CONCURRENCY,
            http_request_timeout: DEFAULT_HTTP_REQUEST_TIMEOUT,
            http_max_response_bytes: DEFAULT_HTTP_MAX_RESPONSE_BYTES,
            storage: None,
        }
    }
}

pub struct AppRuntime {
    event_sender: mpsc::Sender<CoreEvent>,
    action_bus: ActionBus,
    shutdown_sender: watch::Sender<bool>,
    notifications: Arc<NotificationService>,
    tasks: Arc<TaskRuntime>,
    worker: Mutex<Option<JoinHandle<()>>>,
    is_shutdown: AtomicBool,
}

impl AppRuntime {
    pub fn start(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        validate_capacity("event", config.event_queue_capacity)?;
        validate_capacity("action", config.action_queue_capacity)?;
        validate_range(
            "http max concurrency",
            config.http_max_concurrency as u128,
            MAX_HTTP_CONCURRENCY as u128,
        )?;
        validate_range(
            "HTTP request timeout milliseconds",
            config.http_request_timeout.as_millis(),
            MAX_HTTP_REQUEST_TIMEOUT.as_millis(),
        )?;
        validate_range(
            "HTTP max response bytes",
            config.http_max_response_bytes as u128,
            MAX_HTTP_RESPONSE_BYTES as u128,
        )?;
        let content = ContentCatalog::load(&config.content_directory)
            .map_err(|error| RuntimeError::Content(error.to_string()))?;
        let http = Arc::new(
            HttpService::new(HttpServiceConfig {
                max_concurrency: config.http_max_concurrency,
                request_timeout: config.http_request_timeout,
                max_response_bytes: config.http_max_response_bytes,
            })
            .map_err(|error| RuntimeError::Http(error.to_string()))?,
        );
        let storage = Arc::new(StorageService::new(config.storage, Arc::clone(&http)));
        let clock = Arc::new(SystemClock);
        let (event_sender, event_receiver) = mpsc::channel(config.event_queue_capacity);
        let (action_bus, action_sender) = ActionBus::new(config.action_queue_capacity);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let tasks = Arc::new(TaskRuntime::start(
            action_sender.clone(),
            shutdown_receiver.clone(),
        ));
        let command_runtime = CommandRuntime::new(
            &content,
            Arc::clone(&http),
            clock,
            Arc::clone(&storage),
            Arc::clone(&tasks),
            config.ffmpeg_path,
        )
        .map_err(|error| RuntimeError::CommandRegistration(error.to_string()))?;
        let notifications = Arc::new(NotificationService::new(
            Arc::clone(&storage),
            http,
            action_sender.clone(),
            shutdown_receiver.clone(),
        ));
        let role_panel = RolePanelService::new(storage);
        let worker = tokio::spawn(run_worker(
            command_runtime,
            role_panel,
            event_receiver,
            action_sender,
            shutdown_receiver,
        ));

        Ok(Self {
            event_sender,
            action_bus,
            shutdown_sender,
            notifications,
            tasks,
            worker: Mutex::new(Some(worker)),
            is_shutdown: AtomicBool::new(false),
        })
    }

    pub async fn submit_event(&self, event: CoreEvent) -> Result<(), RuntimeError> {
        event.validate_version()?;

        if self.is_shutdown.load(Ordering::Acquire) {
            return Err(RuntimeError::ShuttingDown);
        }

        if let CoreEventData::ActionResult { action_id, outcome } = &event.event
            && self.tasks.handle_action_result(action_id, outcome).await
        {
            return Ok(());
        }

        if let CoreEventData::ActionResult { action_id, outcome } = &event.event
            && self
                .notifications
                .handle_action_result(action_id, outcome)
                .await
        {
            return Ok(());
        }

        let mut shutdown_receiver = self.shutdown_sender.subscribe();
        tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown_receiver) => Err(RuntimeError::ShuttingDown),
            result = self.event_sender.send(event) => {
                result.map_err(|_| RuntimeError::EventQueueClosed)
            }
        }
    }

    pub async fn prepare_notifications(&self) -> Result<(), NotificationError> {
        self.notifications.prepare().await
    }

    pub async fn submit_detection(
        &self,
        value: serde_json::Value,
    ) -> Result<(), NotificationError> {
        self.notifications.submit(value).await
    }

    pub async fn next_action(&self) -> Result<Option<CoreAction>, RuntimeError> {
        if self.is_shutdown.load(Ordering::Acquire) {
            return Ok(None);
        }

        let mut shutdown_receiver = self.shutdown_sender.subscribe();
        tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown_receiver) => Ok(None),
            action = self.action_bus.next_action() => match action {
                Some(action) => Ok(Some(action)),
                None if self.is_shutdown.load(Ordering::Acquire) => Ok(None),
                None => Err(RuntimeError::ActionQueueClosed),
            }
        }
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        if !self.is_shutdown.swap(true, Ordering::AcqRel) {
            self.shutdown_sender.send_replace(true);
        }

        let mut worker = self.worker.lock().await;
        if let Some(worker) = worker.take() {
            worker
                .await
                .map_err(|error| RuntimeError::WorkerJoin(error.to_string()))?;
        }
        self.notifications
            .shutdown()
            .await
            .map_err(|error| RuntimeError::NotificationWorkerJoin(error.to_string()))?;
        self.tasks.shutdown().await?;

        Ok(())
    }

    pub fn is_shutdown(&self) -> bool {
        self.is_shutdown.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    InvalidQueueCapacity {
        queue: &'static str,
        capacity: usize,
        maximum: usize,
    },
    InvalidConfiguration {
        setting: &'static str,
        value: u128,
        maximum: u128,
    },
    ProtocolVersion(ProtocolVersionMismatch),
    Content(String),
    CommandRegistration(String),
    Http(String),
    ShuttingDown,
    EventQueueClosed,
    ActionQueueClosed,
    WorkerJoin(String),
    NotificationWorkerJoin(String),
    TaskWorkerJoin(String),
}

impl Display for RuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidQueueCapacity {
                queue,
                capacity,
                maximum,
            } => write!(
                formatter,
                "{queue} queue capacity must be between 1 and {maximum}, received {capacity}"
            ),
            Self::InvalidConfiguration {
                setting,
                value,
                maximum,
            } => write!(
                formatter,
                "{setting} must be between 1 and {maximum}, received {value}"
            ),
            Self::ProtocolVersion(error) => Display::fmt(error, formatter),
            Self::Content(reason) => write!(formatter, "content initialization failed: {reason}"),
            Self::CommandRegistration(reason) => {
                write!(formatter, "command registration failed: {reason}")
            }
            Self::Http(reason) => write!(formatter, "HTTP service initialization failed: {reason}"),
            Self::ShuttingDown => formatter.write_str("runtime is shutting down"),
            Self::EventQueueClosed => formatter.write_str("event queue is closed"),
            Self::ActionQueueClosed => formatter.write_str("action queue is closed"),
            Self::WorkerJoin(reason) => write!(formatter, "runtime worker failed: {reason}"),
            Self::NotificationWorkerJoin(reason) => {
                write!(formatter, "notification worker failed: {reason}")
            }
            Self::TaskWorkerJoin(reason) => write!(formatter, "task worker failed: {reason}"),
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ProtocolVersion(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProtocolVersionMismatch> for RuntimeError {
    fn from(error: ProtocolVersionMismatch) -> Self {
        Self::ProtocolVersion(error)
    }
}

fn validate_capacity(queue: &'static str, capacity: usize) -> Result<(), RuntimeError> {
    if (1..=MAX_QUEUE_CAPACITY).contains(&capacity) {
        return Ok(());
    }

    Err(RuntimeError::InvalidQueueCapacity {
        queue,
        capacity,
        maximum: MAX_QUEUE_CAPACITY,
    })
}

fn validate_range(setting: &'static str, value: u128, maximum: u128) -> Result<(), RuntimeError> {
    if (1..=maximum).contains(&value) {
        return Ok(());
    }

    Err(RuntimeError::InvalidConfiguration {
        setting,
        value,
        maximum,
    })
}

async fn run_worker(
    command_runtime: CommandRuntime,
    role_panel: RolePanelService,
    mut event_receiver: mpsc::Receiver<CoreEvent>,
    action_sender: mpsc::Sender<CoreAction>,
    mut shutdown_receiver: watch::Receiver<bool>,
) {
    let mut session_manager = SessionManager::new(DEFAULT_MAX_SESSIONS);
    loop {
        let next_expiration = session_manager.next_expiration();
        let event = tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown_receiver) => return,
            _ = wait_for_session_expiration(next_expiration) => {
                let expirations = session_manager.expire(Instant::now());
                for expiration in expirations {
                    let actions = CommandActionBatch::from_data(
                        EventId::new(format!("session:{}:timeout", expiration.session_id)),
                        expiration.request_id,
                        expiration.actions,
                    );
                    if !send_actions(actions, &action_sender, &mut shutdown_receiver).await {
                        return;
                    }
                }
                continue;
            }
            event = event_receiver.recv() => event,
        };
        let Some(event) = event else {
            return;
        };
        let actions = if matches!(&event.event, CoreEventData::MessageCreate { .. }) {
            let mut actions = tokio::select! {
                biased;
                _ = wait_for_shutdown(&mut shutdown_receiver) => return,
                actions = command_runtime.handle_event(event) => actions,
            };
            if let Some(registration) = actions.as_mut().and_then(CommandActionBatch::take_session)
                && let Err(registration) = session_manager.register(registration, Instant::now())
            {
                actions = Some(CommandActionBatch::single(
                    (*registration).into_unavailable_action(),
                ));
            }
            actions
        } else if let Some(action) = role_panel_action(&role_panel, &event).await {
            Some(CommandActionBatch::from_data(
                event.event_id.clone(),
                event.request_id.clone(),
                vec![action],
            ))
        } else {
            handle_session_event(&mut session_manager, event, Instant::now()).await
        };
        let Some(actions) = actions else {
            continue;
        };

        if !send_actions(actions, &action_sender, &mut shutdown_receiver).await {
            return;
        }
    }
}

async fn role_panel_action(
    role_panel: &RolePanelService,
    event: &CoreEvent,
) -> Option<kbc_protocol::CoreActionData> {
    match &event.event {
        CoreEventData::ReactionAdd {
            guild_id,
            channel_id,
            message_id,
            user_id,
            emoji,
        } => {
            role_panel
                .action_for_reaction(
                    guild_id.as_deref(),
                    channel_id,
                    message_id,
                    user_id,
                    emoji,
                    true,
                )
                .await
        }
        CoreEventData::ReactionRemove {
            guild_id,
            channel_id,
            message_id,
            user_id,
            emoji,
        } => {
            role_panel
                .action_for_reaction(
                    guild_id.as_deref(),
                    channel_id,
                    message_id,
                    user_id,
                    emoji,
                    false,
                )
                .await
        }
        _ => None,
    }
}

async fn handle_session_event(
    session_manager: &mut SessionManager,
    event: CoreEvent,
    now: Instant,
) -> Option<CommandActionBatch> {
    let CoreEvent {
        event_id, event, ..
    } = event;
    let output = match event {
        CoreEventData::ActionResult { action_id, outcome } => {
            session_manager.handle_action_result(&action_id, outcome, now)
        }
        CoreEventData::ReactionAdd {
            channel_id,
            message_id,
            user_id,
            emoji,
            ..
        } => {
            session_manager
                .handle_reaction(&channel_id, &message_id, &user_id, &emoji, now)
                .await
        }
        CoreEventData::ReactionRemove { .. } => None,
        CoreEventData::MessageCreate { .. } => None,
    }?;

    let activation = output.activation;
    let session = output.session;
    let request_id = output.request_id;
    let actions = CommandActionBatch::from_data(event_id, request_id.clone(), output.actions);
    if let Some(activation) = activation
        && let Some(action_id) = actions.last_action_id()
    {
        session_manager.register_activation(action_id, activation, now);
    }
    if let Some(session) = session
        && let Some(action_id) = actions.last_action_id()
        && session_manager
            .register(
                SessionRegistration::new(action_id, request_id, session),
                now,
            )
            .is_err()
    {
        eprintln!("A continued session could not be registered");
    }
    Some(actions)
}

async fn send_actions(
    actions: CommandActionBatch,
    action_sender: &mpsc::Sender<CoreAction>,
    shutdown_receiver: &mut watch::Receiver<bool>,
) -> bool {
    for action in actions {
        let sent = tokio::select! {
            biased;
            _ = wait_for_shutdown(shutdown_receiver) => false,
            result = action_sender.send(action) => result.is_ok(),
        };
        if !sent {
            return false;
        }
    }
    true
}

async fn wait_for_session_expiration(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
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
