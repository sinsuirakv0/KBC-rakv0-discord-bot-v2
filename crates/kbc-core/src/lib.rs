//! Discord非依存のBot RuntimeとDomain Logicを置くcrate。

mod action_bus;
mod command;
mod command_runtime;
mod commands;
mod content;
mod motion;
mod notification;
mod runtime;
mod services;
mod session;
mod storage;
mod task_runtime;

pub use notification::NotificationError;
pub use runtime::{
    AppRuntime, DEFAULT_ACTION_QUEUE_CAPACITY, DEFAULT_EVENT_QUEUE_CAPACITY,
    DEFAULT_HTTP_MAX_CONCURRENCY, DEFAULT_HTTP_MAX_RESPONSE_BYTES, DEFAULT_HTTP_REQUEST_TIMEOUT,
    MAX_QUEUE_CAPACITY, RuntimeConfig, RuntimeError,
};
pub use storage::StorageConfig;
