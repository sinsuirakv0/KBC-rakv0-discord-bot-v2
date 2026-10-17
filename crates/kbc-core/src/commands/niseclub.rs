use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::services::Clock;
use kbc_protocol::CoreActionData;
use std::sync::Arc;

pub(super) struct NiseClubCommand {
    metadata: CommandMetadata,
    help: String,
    clock: Arc<dyn Clock>,
}
impl NiseClubCommand {
    pub(super) fn new(help: &str, clock: Arc<dyn Clock>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "niseclub".into(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.into(),
            clock,
        }
    }
}
impl Command for NiseClubCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }
    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        let content = if !arguments.is_empty() {
            self.help.clone()
        } else {
            crate::url_key::club_link(self.clock.now().timestamp_millis()).unwrap_or_else(|| {
                "URLkeyの生成設定が未設定です。Bot管理者に連絡してください。".into()
            })
        };
        Box::pin(async move {
            Ok(CommandOutput::single(CoreActionData::SendMessage {
                channel_id: context.into_channel_id(),
                content,
            }))
        })
    }
}
