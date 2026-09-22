//! CoreEventをCommandへdispatchし、CoreActionへ変換する。

use std::path::PathBuf;
use std::sync::Arc;

use kbc_protocol::{
    ActionId, CoreAction, CoreActionData, CoreEvent, CoreEventData, EventId, PROTOCOL_VERSION,
    RequestId,
};

use crate::command::{CommandContext, CommandOutput, CommandRegistrationError, CommandRegistry};
use crate::commands::{MissingCommandContent, built_in_commands};
use crate::content::ContentCatalog;
use crate::services::{Clock, HttpService};
use crate::session::SessionRegistration;
use crate::storage::StorageService;
use crate::task_runtime::TaskRuntime;

const COMMAND_PREFIX: &str = "o.";

pub(crate) struct CommandRuntime {
    registry: CommandRegistry,
}

impl CommandRuntime {
    pub(crate) fn new(
        content: &ContentCatalog,
        http: Arc<HttpService>,
        clock: Arc<dyn Clock>,
        storage: Arc<StorageService>,
        tasks: Arc<TaskRuntime>,
        ffmpeg_path: Option<PathBuf>,
    ) -> Result<Self, CommandRuntimeInitializationError> {
        Ok(Self {
            registry: CommandRegistry::new(built_in_commands(
                content,
                http,
                clock,
                storage,
                tasks,
                ffmpeg_path,
            )?)?,
        })
    }

    pub(crate) async fn handle_event(&self, event: CoreEvent) -> Option<CommandActionBatch> {
        let CoreEvent {
            event_id,
            request_id,
            event,
            ..
        } = event;
        let CoreEventData::MessageCreate {
            guild_id,
            channel_id,
            user_id,
            content,
            ..
        } = event
        else {
            return None;
        };
        let input = parse_command_input(&content, COMMAND_PREFIX)?;
        let command = self.registry.resolve(&input.name)?;
        if command.metadata().guild_only && guild_id.is_none() {
            return None;
        }

        let context =
            CommandContext::new(guild_id, channel_id.clone(), user_id, request_id.clone());
        let output = match command.execute(context, input.arguments).await {
            Ok(output) => output,
            Err(error) => {
                eprintln!("Command execution failed: {error}");
                CommandOutput::single(kbc_protocol::CoreActionData::SendMessage {
                    channel_id,
                    content: "❌ コマンドの実行に失敗しました".to_owned(),
                })
            }
        };
        let (action_data, session) = output.into_parts();
        let actions = action_data
            .into_iter()
            .enumerate()
            .map(|(index, action)| CoreAction {
                protocol_version: PROTOCOL_VERSION,
                action_id: ActionId::new(action_id(event_id.as_str(), index)),
                request_id: request_id.clone(),
                action,
            })
            .collect::<Vec<_>>();
        let session = session.and_then(|request| {
            actions.first().map(|action| {
                SessionRegistration::new(action.action_id.clone(), request_id.clone(), request)
            })
        });
        Some(CommandActionBatch { actions, session })
    }
}

pub(crate) struct CommandActionBatch {
    actions: Vec<CoreAction>,
    session: Option<SessionRegistration>,
}

impl CommandActionBatch {
    pub(crate) fn from_data(
        event_id: EventId,
        request_id: RequestId,
        actions: Vec<CoreActionData>,
    ) -> Self {
        let actions = actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| CoreAction {
                protocol_version: PROTOCOL_VERSION,
                action_id: ActionId::new(action_id(event_id.as_str(), index)),
                request_id: request_id.clone(),
                action,
            })
            .collect();
        Self {
            actions,
            session: None,
        }
    }

    pub(crate) fn single(action: CoreAction) -> Self {
        Self {
            actions: vec![action],
            session: None,
        }
    }

    pub(crate) fn take_session(&mut self) -> Option<SessionRegistration> {
        self.session.take()
    }

    pub(crate) fn last_action_id(&self) -> Option<ActionId> {
        self.actions.last().map(|action| action.action_id.clone())
    }
}

impl IntoIterator for CommandActionBatch {
    type Item = CoreAction;
    type IntoIter = std::vec::IntoIter<CoreAction>;

    fn into_iter(self) -> Self::IntoIter {
        self.actions.into_iter()
    }
}

fn action_id(event_id: &str, index: usize) -> String {
    if index == 0 {
        format!("action:{event_id}")
    } else {
        format!("action:{event_id}:{}", index + 1)
    }
}

#[derive(Debug)]
pub(crate) enum CommandRuntimeInitializationError {
    MissingContent(MissingCommandContent),
    Registration(CommandRegistrationError),
}

impl std::fmt::Display for CommandRuntimeInitializationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingContent(error) => std::fmt::Display::fmt(error, formatter),
            Self::Registration(error) => std::fmt::Display::fmt(error, formatter),
        }
    }
}

impl From<MissingCommandContent> for CommandRuntimeInitializationError {
    fn from(error: MissingCommandContent) -> Self {
        Self::MissingContent(error)
    }
}

impl From<CommandRegistrationError> for CommandRuntimeInitializationError {
    fn from(error: CommandRegistrationError) -> Self {
        Self::Registration(error)
    }
}

struct ParsedCommandInput {
    name: String,
    arguments: Vec<String>,
}

fn parse_command_input(content: &str, prefix: &str) -> Option<ParsedCommandInput> {
    let command_text = content.strip_prefix(prefix)?.trim();
    let mut parts = command_text.split_whitespace();
    let name = parts.next()?.to_ascii_lowercase();

    Some(ParsedCommandInput {
        name,
        arguments: parts.map(str::to_owned).collect(),
    })
}
