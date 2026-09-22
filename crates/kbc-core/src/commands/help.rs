//! Command一覧を表示するhelp Command。

use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};

pub(super) struct HelpCommand {
    metadata: CommandMetadata,
    index: String,
}

impl HelpCommand {
    pub(super) fn new(index: &str) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "help".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            index: index.to_owned(),
        }
    }
}

impl Command for HelpCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(
        &'a self,
        context: CommandContext,
        _arguments: Vec<String>,
    ) -> CommandFuture<'a> {
        let content = self.index.clone();
        Box::pin(async move {
            Ok(CommandOutput::single(CoreActionData::SendMessage {
                channel_id: context.into_channel_id(),
                content,
            }))
        })
    }
}
