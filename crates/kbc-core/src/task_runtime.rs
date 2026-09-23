//! 長時間処理の有限Queue、進捗、Action結果、File cleanupを管理する。

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use kbc_protocol::{
    ActionId, ActionOutcome, CoreAction, CoreActionData, PROTOCOL_VERSION, RequestId,
};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tokio::task::{Id as TokioTaskId, JoinError, JoinHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout};
use tokio_util::sync::CancellationToken;

use crate::runtime::RuntimeError;

const TASK_CAPACITY: usize = 2;
const ACTIVE_TASKS: usize = 1;
const QUEUE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);
const CANCELLATION_GRACE: Duration = Duration::from_secs(15);
const MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) type TaskFuture =
    Pin<Box<dyn Future<Output = Result<TaskArtifact, TaskFailure>> + Send>>;

pub(crate) trait TaskJob: Send + 'static {
    fn run(self: Box<Self>, context: TaskContext) -> TaskFuture;
}

pub(crate) struct TaskArtifact {
    path: PathBuf,
    file_name: String,
    content_type: Option<String>,
    message: Option<String>,
}

impl TaskArtifact {
    pub(crate) fn new(
        path: PathBuf,
        file_name: impl Into<String>,
        content_type: Option<String>,
        message: Option<String>,
    ) -> Self {
        Self {
            path,
            file_name: file_name.into(),
            content_type,
            message,
        }
    }
}

#[derive(Debug)]
pub(crate) struct TaskFailure {
    public_message: String,
    detail: String,
}

impl TaskFailure {
    pub(crate) fn new(public_message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            public_message: public_message.into(),
            detail: detail.into(),
        }
    }
}

impl Display for TaskFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

#[derive(Clone)]
pub(crate) struct TaskContext {
    workspace: Arc<PathBuf>,
    progress_sender: watch::Sender<String>,
    cancellation: CancellationToken,
    queue_wait: Duration,
}

impl TaskContext {
    pub(crate) fn output_path(&self, file_name: &str) -> Result<PathBuf, TaskFailure> {
        let path = Path::new(file_name);
        if file_name.is_empty()
            || path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
        {
            return Err(TaskFailure::new(
                "❌ 出力ファイルを作成できませんでした",
                "invalid task output file name",
            ));
        }
        Ok(self.workspace.join(path))
    }

    pub(crate) fn report(&self, content: impl Into<String>) {
        self.progress_sender.send_replace(content.into());
    }

    pub(crate) fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    pub(crate) fn queue_wait(&self) -> Duration {
        self.queue_wait
    }
}

pub(crate) struct TaskSubmission {
    request_id: RequestId,
    channel_id: String,
    initial_message: String,
    job: Box<dyn TaskJob>,
}

impl TaskSubmission {
    pub(crate) fn new(
        request_id: RequestId,
        channel_id: String,
        initial_message: impl Into<String>,
        job: Box<dyn TaskJob>,
    ) -> Self {
        Self {
            request_id,
            channel_id,
            initial_message: initial_message.into(),
            job,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskSubmitError {
    Busy,
    Unavailable,
}

impl Display for TaskSubmitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => formatter.write_str("task queue is full"),
            Self::Unavailable => formatter.write_str("task runtime is unavailable"),
        }
    }
}

struct QueuedTask {
    id: u64,
    submission: TaskSubmission,
    _slot: OwnedSemaphorePermit,
}

struct TaskShared {
    action_sender: mpsc::Sender<CoreAction>,
    action_waiters: Mutex<HashMap<ActionId, oneshot::Sender<ActionOutcome>>>,
    active: Arc<Semaphore>,
    next_action_sequence: AtomicU64,
    shutdown_receiver: watch::Receiver<bool>,
}

pub(crate) struct TaskRuntime {
    submission_sender: mpsc::Sender<QueuedTask>,
    slots: Arc<Semaphore>,
    next_task_id: AtomicU64,
    shared: Arc<TaskShared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl TaskRuntime {
    pub(crate) fn start(
        action_sender: mpsc::Sender<CoreAction>,
        shutdown_receiver: watch::Receiver<bool>,
    ) -> Self {
        let (submission_sender, submission_receiver) = mpsc::channel(TASK_CAPACITY);
        let shared = Arc::new(TaskShared {
            action_sender,
            action_waiters: Mutex::new(HashMap::new()),
            active: Arc::new(Semaphore::new(ACTIVE_TASKS)),
            next_action_sequence: AtomicU64::new(1),
            shutdown_receiver,
        });
        let worker = tokio::spawn(run_scheduler(submission_receiver, Arc::clone(&shared)));
        Self {
            submission_sender,
            slots: Arc::new(Semaphore::new(TASK_CAPACITY)),
            next_task_id: AtomicU64::new(1),
            shared,
            worker: Mutex::new(Some(worker)),
        }
    }

    pub(crate) fn submit(&self, submission: TaskSubmission) -> Result<u64, TaskSubmitError> {
        let slot = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| TaskSubmitError::Busy)?;
        let id = self.next_task_id.fetch_add(1, Ordering::Relaxed);
        self.submission_sender
            .try_send(QueuedTask {
                id,
                submission,
                _slot: slot,
            })
            .map_err(|_| TaskSubmitError::Unavailable)?;
        Ok(id)
    }

    pub(crate) async fn handle_action_result(
        &self,
        action_id: &ActionId,
        outcome: &ActionOutcome,
    ) -> bool {
        let waiter = self.shared.action_waiters.lock().await.remove(action_id);
        if let Some(waiter) = waiter {
            let _ = waiter.send(outcome.clone());
            true
        } else {
            false
        }
    }

    pub(crate) async fn request_adapter(
        &self,
        request_id: &RequestId,
        action: CoreActionData,
    ) -> Result<ActionOutcome, &'static str> {
        perform_action(&self.shared, 0, request_id, action).await
    }

    pub(crate) async fn shutdown(&self) -> Result<(), RuntimeError> {
        let mut worker = self.worker.lock().await;
        if let Some(worker) = worker.take() {
            worker
                .await
                .map_err(|error| RuntimeError::TaskWorkerJoin(error.to_string()))?;
        }
        Ok(())
    }
}

async fn run_scheduler(
    mut submission_receiver: mpsc::Receiver<QueuedTask>,
    shared: Arc<TaskShared>,
) {
    let mut shutdown_receiver = shared.shutdown_receiver.clone();
    let mut tasks = JoinSet::new();
    let mut task_ids = HashMap::new();
    loop {
        tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown_receiver) => break,
            result = tasks.join_next_with_id(), if !tasks.is_empty() => {
                if let Some(result) = result {
                    handle_task_join(result, &mut task_ids).await;
                }
            }
            submission = submission_receiver.recv() => match submission {
                Some(task) => {
                    let task_id = task.id;
                    let shared = Arc::clone(&shared);
                    let abort_handle = tasks.spawn(async move { run_task(task, shared).await });
                    task_ids.insert(abort_handle.id(), task_id);
                }
                None => break,
            }
        }
    }
    while let Some(result) = tasks.join_next_with_id().await {
        handle_task_join(result, &mut task_ids).await;
    }
}

async fn handle_task_join(
    result: Result<(TokioTaskId, ()), JoinError>,
    task_ids: &mut HashMap<TokioTaskId, u64>,
) {
    match result {
        Ok((tokio_id, ())) => {
            task_ids.remove(&tokio_id);
        }
        Err(error) => {
            let task_id = task_ids.remove(&error.id());
            let failure = if error.is_panic() {
                "panicked"
            } else if error.is_cancelled() {
                "was cancelled"
            } else {
                "failed to join"
            };
            if let Some(task_id) = task_id {
                eprintln!("Task {task_id} {failure}: {error}");
                cleanup_task_workspace(task_id).await;
            } else {
                eprintln!("Unknown task {failure}: {error}");
            }
        }
    }
}

async fn run_task(task: QueuedTask, shared: Arc<TaskShared>) {
    let QueuedTask {
        id,
        submission,
        _slot,
    } = task;
    let TaskSubmission {
        request_id,
        channel_id,
        initial_message,
        job,
    } = submission;
    let initial = perform_action(
        &shared,
        id,
        &request_id,
        CoreActionData::SendMessage {
            channel_id: channel_id.clone(),
            content: initial_message,
        },
    )
    .await;
    let Ok(ActionOutcome::Success {
        message_id: Some(message_id),
    }) = initial
    else {
        return;
    };

    let queue_started = Instant::now();
    let active = match timeout(QUEUE_TIMEOUT, Arc::clone(&shared.active).acquire_owned()).await {
        Ok(Ok(active)) => active,
        Ok(Err(_)) => return,
        Err(_) => {
            emit_terminal(
                &shared,
                id,
                &request_id,
                &channel_id,
                &message_id,
                "❌ 待機時間が上限を超えました。もう一度お試しください",
            )
            .await;
            return;
        }
    };
    let queue_wait = queue_started.elapsed();

    let workspace = task_workspace(id);
    if let Err(error) = tokio::fs::create_dir_all(&workspace).await {
        emit_terminal(
            &shared,
            id,
            &request_id,
            &channel_id,
            &message_id,
            "❌ 作業用ファイルを準備できませんでした",
        )
        .await;
        eprintln!("Task {id} workspace creation failed: {error}");
        cleanup_task_workspace(id).await;
        drop(active);
        return;
    }

    let cancellation = CancellationToken::new();
    let (progress_sender, progress_receiver) = watch::channel(String::new());
    let context = TaskContext {
        workspace: Arc::new(workspace.clone()),
        progress_sender,
        cancellation: cancellation.clone(),
        queue_wait,
    };
    let result = supervise_job(
        job.run(context),
        progress_receiver,
        cancellation,
        &shared,
        id,
        &request_id,
        &channel_id,
        &message_id,
    )
    .await;

    match result {
        Ok(artifact) => {
            if let Err(error) =
                send_artifact(&shared, id, &request_id, &channel_id, &workspace, artifact).await
            {
                eprintln!("Task {id} attachment failed: {error}");
                emit_terminal(
                    &shared,
                    id,
                    &request_id,
                    &channel_id,
                    &message_id,
                    "❌ 生成したファイルを送信できませんでした",
                )
                .await;
            } else {
                emit_terminal(
                    &shared,
                    id,
                    &request_id,
                    &channel_id,
                    &message_id,
                    "✅ 生成が完了しました",
                )
                .await;
            }
        }
        Err(failure) => {
            eprintln!("Task {id} failed: {}", failure.detail);
            emit_terminal(
                &shared,
                id,
                &request_id,
                &channel_id,
                &message_id,
                &failure.public_message,
            )
            .await;
        }
    }

    cleanup_task_workspace(id).await;
    drop(active);
}

#[allow(clippy::too_many_arguments)]
async fn supervise_job(
    mut job: TaskFuture,
    mut progress_receiver: watch::Receiver<String>,
    cancellation: CancellationToken,
    shared: &TaskShared,
    task_id: u64,
    request_id: &RequestId,
    channel_id: &str,
    message_id: &str,
) -> Result<TaskArtifact, TaskFailure> {
    let execution_deadline = Instant::now() + EXECUTION_TIMEOUT;
    let execution_timeout = sleep_until(execution_deadline);
    tokio::pin!(execution_timeout);
    let stall_timeout = sleep_until(Instant::now() + STALL_TIMEOUT);
    tokio::pin!(stall_timeout);
    let mut progress_tick = interval_at(Instant::now() + PROGRESS_INTERVAL, PROGRESS_INTERVAL);
    progress_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_sent = String::new();
    let mut progress_closed = false;
    let mut shutdown_receiver = shared.shutdown_receiver.clone();

    loop {
        tokio::select! {
            biased;
            _ = wait_for_shutdown(&mut shutdown_receiver) => {
                return cancel_and_wait(
                    &mut job,
                    &cancellation,
                    TaskFailure::new("❌ 処理を中断しました", "runtime shutdown"),
                ).await;
            }
            _ = &mut execution_timeout => {
                return cancel_and_wait(
                    &mut job,
                    &cancellation,
                    TaskFailure::new("❌ 処理時間が上限を超えました", "execution timeout"),
                ).await;
            }
            _ = &mut stall_timeout => {
                return cancel_and_wait(
                    &mut job,
                    &cancellation,
                    TaskFailure::new("❌ 処理の進行が停止したため中断しました", "stall timeout"),
                ).await;
            }
            changed = progress_receiver.changed(), if !progress_closed => {
                match changed {
                    Ok(()) => stall_timeout.as_mut().reset(Instant::now() + STALL_TIMEOUT),
                    Err(_) => progress_closed = true,
                }
            }
            _ = progress_tick.tick() => {
                let content = progress_receiver.borrow().clone();
                if !content.is_empty() && content != last_sent {
                    let action = core_action(
                        shared,
                        task_id,
                        request_id,
                        CoreActionData::EditMessage {
                            channel_id: channel_id.to_owned(),
                            message_id: message_id.to_owned(),
                            content: content.clone(),
                        },
                    );
                    if shared.action_sender.try_send(action).is_ok() {
                        last_sent = content;
                    }
                }
            }
            result = &mut job => return result,
        }
    }
}

async fn cancel_and_wait(
    job: &mut TaskFuture,
    cancellation: &CancellationToken,
    failure: TaskFailure,
) -> Result<TaskArtifact, TaskFailure> {
    cancellation.cancel();
    let _ = timeout(CANCELLATION_GRACE, &mut *job).await;
    Err(failure)
}

async fn send_artifact(
    shared: &TaskShared,
    task_id: u64,
    request_id: &RequestId,
    channel_id: &str,
    workspace: &Path,
    artifact: TaskArtifact,
) -> Result<(), String> {
    if artifact.path.parent() != Some(workspace) {
        return Err("artifact path is outside its task workspace".to_owned());
    }
    let metadata = tokio::fs::metadata(&artifact.path)
        .await
        .map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!("invalid artifact size: {}", metadata.len()));
    }
    let outcome = perform_action(
        shared,
        task_id,
        request_id,
        CoreActionData::SendAttachmentFile {
            channel_id: channel_id.to_owned(),
            file_name: artifact.file_name,
            content_type: artifact.content_type,
            message: artifact.message,
            path: artifact.path.to_string_lossy().into_owned(),
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    if matches!(outcome, ActionOutcome::Success { .. }) {
        Ok(())
    } else {
        Err("Discord rejected the attachment action".to_owned())
    }
}

async fn emit_terminal(
    shared: &TaskShared,
    task_id: u64,
    request_id: &RequestId,
    channel_id: &str,
    message_id: &str,
    content: &str,
) {
    let action = core_action(
        shared,
        task_id,
        request_id,
        CoreActionData::EditMessage {
            channel_id: channel_id.to_owned(),
            message_id: message_id.to_owned(),
            content: content.to_owned(),
        },
    );
    let mut shutdown_receiver = shared.shutdown_receiver.clone();
    tokio::select! {
        biased;
        _ = wait_for_shutdown(&mut shutdown_receiver) => {}
        _ = shared.action_sender.send(action) => {}
    }
}

async fn perform_action(
    shared: &TaskShared,
    task_id: u64,
    request_id: &RequestId,
    action: CoreActionData,
) -> Result<ActionOutcome, &'static str> {
    let action = core_action(shared, task_id, request_id, action);
    let action_id = action.action_id.clone();
    let (sender, receiver) = oneshot::channel();
    shared
        .action_waiters
        .lock()
        .await
        .insert(action_id.clone(), sender);
    let mut shutdown_receiver = shared.shutdown_receiver.clone();
    let sent = tokio::select! {
        biased;
        _ = wait_for_shutdown(&mut shutdown_receiver) => false,
        result = shared.action_sender.send(action) => result.is_ok(),
    };
    if !sent {
        shared.action_waiters.lock().await.remove(&action_id);
        return Err("action queue is unavailable");
    }
    let outcome = tokio::select! {
        biased;
        _ = wait_for_shutdown(&mut shutdown_receiver) => Err("runtime shutdown"),
        result = timeout(ACTION_TIMEOUT, receiver) => match result {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(_)) => Err("action result channel closed"),
            Err(_) => Err("action result timeout"),
        },
    };
    if outcome.is_err() {
        shared.action_waiters.lock().await.remove(&action_id);
    }
    outcome
}

fn core_action(
    shared: &TaskShared,
    task_id: u64,
    request_id: &RequestId,
    action: CoreActionData,
) -> CoreAction {
    let sequence = shared.next_action_sequence.fetch_add(1, Ordering::Relaxed);
    CoreAction {
        protocol_version: PROTOCOL_VERSION,
        action_id: ActionId::new(format!("task:{task_id}:{sequence}")),
        request_id: request_id.clone(),
        action,
    }
}

fn task_workspace(task_id: u64) -> PathBuf {
    std::env::temp_dir()
        .join("kbc-bot-v2")
        .join(format!("task-{}-{task_id}", std::process::id()))
}

async fn cleanup_task_workspace(task_id: u64) {
    if let Err(error) = tokio::fs::remove_dir_all(task_workspace(task_id)).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("Task {task_id} workspace cleanup failed: {error}");
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct TestJob;

    impl TaskJob for TestJob {
        fn run(self: Box<Self>, context: TaskContext) -> TaskFuture {
            Box::pin(async move {
                let path = context.output_path("test.txt")?;
                tokio::fs::write(&path, b"test")
                    .await
                    .map_err(|error| TaskFailure::new("test output failed", error.to_string()))?;
                Ok(TaskArtifact::new(
                    path,
                    "test.txt",
                    Some("text/plain".to_owned()),
                    None,
                ))
            })
        }
    }

    async fn receive_action(receiver: &mut mpsc::Receiver<CoreAction>) -> CoreAction {
        timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("Action受信がtimeoutしました")
            .expect("Action channelが閉じました")
    }

    #[tokio::test]
    async fn waits_for_attachment_result_before_cleanup() {
        let (action_sender, mut action_receiver) = mpsc::channel(8);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let runtime = TaskRuntime::start(action_sender, shutdown_receiver);

        let request_id = RequestId::new("request:test-task");
        assert_eq!(
            runtime
                .submit(TaskSubmission::new(
                    request_id,
                    "channel:test".to_owned(),
                    "processing",
                    Box::new(TestJob),
                ))
                .expect("Task登録に失敗しました"),
            1
        );

        let initial = receive_action(&mut action_receiver).await;
        let initial_action_id = initial.action_id.clone();
        assert!(matches!(initial.action, CoreActionData::SendMessage { .. }));
        assert!(
            runtime
                .handle_action_result(
                    &initial_action_id,
                    &ActionOutcome::Success {
                        message_id: Some("message:test".to_owned()),
                    },
                )
                .await
        );

        let attachment = receive_action(&mut action_receiver).await;
        let attachment_action_id = attachment.action_id.clone();
        let attachment_path = match &attachment.action {
            CoreActionData::SendAttachmentFile { path, .. } => path.clone(),
            action => panic!("unexpected attachment action: {action:?}"),
        };
        assert!(
            runtime
                .handle_action_result(
                    &attachment_action_id,
                    &ActionOutcome::Success { message_id: None },
                )
                .await
        );

        let terminal = receive_action(&mut action_receiver).await;
        assert!(matches!(
            terminal.action,
            CoreActionData::EditMessage { content, .. } if content == "✅ 生成が完了しました"
        ));
        assert!(Path::new(&attachment_path).exists());

        shutdown_sender
            .send(true)
            .expect("shutdown通知に失敗しました");
        runtime
            .shutdown()
            .await
            .expect("Task Runtime停止に失敗しました");
        assert!(!Path::new(&attachment_path).exists());
    }

    #[tokio::test]
    async fn cancellation_waits_for_job_to_observe_token() {
        let cancellation = CancellationToken::new();
        let observed = Arc::new(AtomicBool::new(false));
        let job_observed = Arc::clone(&observed);
        let job_cancellation = cancellation.clone();
        let mut job: TaskFuture = Box::pin(async move {
            job_cancellation.cancelled().await;
            job_observed.store(true, Ordering::SeqCst);
            Err(TaskFailure::new("cancelled", "test cancellation"))
        });

        let result = cancel_and_wait(
            &mut job,
            &cancellation,
            TaskFailure::new("cancelled", "test timeout"),
        )
        .await;

        assert!(result.is_err());
        assert!(cancellation.is_cancelled());
        assert!(observed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn closed_progress_channel_does_not_starve_job() {
        let (action_sender, _action_receiver) = mpsc::channel(1);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let shared = TaskShared {
            action_sender,
            action_waiters: Mutex::new(HashMap::new()),
            active: Arc::new(Semaphore::new(1)),
            next_action_sequence: AtomicU64::new(1),
            shutdown_receiver,
        };
        let cancellation = CancellationToken::new();
        let (progress_sender, progress_receiver) = watch::channel(String::new());
        drop(progress_sender);
        let job: TaskFuture = Box::pin(async {
            tokio::task::yield_now().await;
            Err(TaskFailure::new("finished", "test job finished"))
        });

        let result = timeout(
            Duration::from_secs(1),
            supervise_job(
                job,
                progress_receiver,
                cancellation,
                &shared,
                1,
                &RequestId::new("request:closed-progress"),
                "channel:test",
                "message:test",
            ),
        )
        .await
        .expect("progress channel切断後にjobが完了しませんでした");

        let Err(result) = result else {
            panic!("test jobは失敗を返す必要があります");
        };

        assert_eq!(result.detail, "test job finished");
    }

    #[tokio::test]
    async fn panic_is_reported_with_task_id_and_workspace_is_removed() {
        let task_id = u64::MAX;
        cleanup_task_workspace(task_id).await;
        let workspace = task_workspace(task_id);
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("test workspaceを作成できませんでした");
        tokio::fs::write(workspace.join("artifact"), b"test")
            .await
            .expect("test artifactを作成できませんでした");
        let mut tasks = JoinSet::new();
        let abort_handle = tasks.spawn(async { panic!("test task panic") });
        let mut task_ids = HashMap::from([(abort_handle.id(), task_id)]);

        let result = tasks
            .join_next_with_id()
            .await
            .expect("panicしたtaskのJoin結果がありません");
        handle_task_join(result, &mut task_ids).await;

        assert!(task_ids.is_empty());
        assert!(!workspace.exists());
    }
}
