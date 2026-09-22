//! Command実装が共有する最小APIとRegistryを定義する。

use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;

use kbc_protocol::CoreActionData;

use crate::session::SessionRequest;

pub(crate) const MAX_COMMAND_ACTIONS: usize = 32;

pub(crate) type CommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CommandOutput, CommandExecutionError>> + Send + 'a>>;

pub(crate) struct CommandOutput {
    actions: Vec<CoreActionData>,
    session: Option<SessionRequest>,
}

impl CommandOutput {
    pub(crate) fn new() -> Self {
        Self {
            actions: Vec::new(),
            session: None,
        }
    }

    pub(crate) fn single(action: CoreActionData) -> Self {
        Self {
            actions: vec![action],
            session: None,
        }
    }

    pub(crate) fn with_session(action: CoreActionData, session: SessionRequest) -> Self {
        Self {
            actions: vec![action],
            session: Some(session),
        }
    }

    pub(crate) fn push(&mut self, action: CoreActionData) -> Result<(), CommandExecutionError> {
        if self.actions.len() >= MAX_COMMAND_ACTIONS {
            return Err(CommandExecutionError::TooManyActions);
        }
        self.actions.push(action);
        Ok(())
    }

    pub(crate) fn into_parts(self) -> (Vec<CoreActionData>, Option<SessionRequest>) {
        (self.actions, self.session)
    }

    pub(crate) fn into_actions(self) -> impl Iterator<Item = CoreActionData> {
        self.actions.into_iter()
    }
}

#[derive(Debug)]
pub(crate) enum CommandExecutionError {
    TooManyActions,
    InvalidSession,
}

impl Display for CommandExecutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyActions => write!(
                formatter,
                "command produced more than {MAX_COMMAND_ACTIONS} actions"
            ),
            Self::InvalidSession => formatter.write_str("command produced an invalid session"),
        }
    }
}

pub(crate) struct CommandMetadata {
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) guild_only: bool,
}

pub(crate) struct CommandContext {
    guild_id: Option<String>,
    channel_id: String,
    user_id: String,
}

impl CommandContext {
    pub(crate) fn new(guild_id: Option<String>, channel_id: String, user_id: String) -> Self {
        Self {
            guild_id,
            channel_id,
            user_id,
        }
    }

    pub(crate) fn guild_id(&self) -> Option<&str> {
        self.guild_id.as_deref()
    }

    pub(crate) fn into_channel_id(self) -> String {
        self.channel_id
    }

    pub(crate) fn channel_id(&self) -> &str {
        &self.channel_id
    }

    pub(crate) fn user_id(&self) -> &str {
        &self.user_id
    }
}

pub(crate) trait Command: Send + Sync {
    fn metadata(&self) -> &CommandMetadata;

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a>;
}

pub(crate) struct CommandRegistry {
    commands: Vec<Box<dyn Command>>,
    indexes_by_name: HashMap<String, usize>,
}

impl CommandRegistry {
    pub(crate) fn new(commands: Vec<Box<dyn Command>>) -> Result<Self, CommandRegistrationError> {
        let mut registered_commands = Vec::with_capacity(commands.len());
        let mut indexes_by_name = HashMap::new();

        for command in commands {
            let index = registered_commands.len();
            let metadata = command.metadata();
            for name in std::iter::once(metadata.name.as_str())
                .chain(metadata.aliases.iter().map(String::as_str))
            {
                let normalized_name = name.to_ascii_lowercase();
                if indexes_by_name.contains_key(&normalized_name) {
                    return Err(CommandRegistrationError {
                        name: normalized_name,
                    });
                }
                indexes_by_name.insert(normalized_name, index);
            }
            registered_commands.push(command);
        }

        Ok(Self {
            commands: registered_commands,
            indexes_by_name,
        })
    }

    pub(crate) fn resolve(&self, normalized_name: &str) -> Option<&dyn Command> {
        let index = *self.indexes_by_name.get(normalized_name)?;
        Some(self.commands[index].as_ref())
    }
}

#[derive(Debug)]
pub(crate) struct CommandRegistrationError {
    name: String,
}

impl Display for CommandRegistrationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "duplicate command registration: {}", self.name)
    }
}

impl Error for CommandRegistrationError {}
