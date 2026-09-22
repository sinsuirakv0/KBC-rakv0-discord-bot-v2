//! Local contentから構築する静的返信Command。

use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};

pub(super) struct StaticResponseCommand {
    metadata: CommandMetadata,
    response: String,
    help: String,
}

impl StaticResponseCommand {
    pub(super) fn new(name: &str, response: &str, help: &str) -> Self {
        Self {
            metadata: CommandMetadata {
                name: name.to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            response: response.to_owned(),
            help: help.to_owned(),
        }
    }
}

impl Command for StaticResponseCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        let content = if arguments
            .first()
            .is_some_and(|argument| argument.eq_ignore_ascii_case("help"))
        {
            self.help.clone()
        } else {
            self.response.clone()
        };

        Box::pin(async move {
            Ok(CommandOutput::single(CoreActionData::SendMessage {
                channel_id: context.into_channel_id(),
                content,
            }))
        })
    }
}
