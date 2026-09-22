//! 通知先をprivate GitHub Repositoryへ保存する`push` Command。

use std::sync::Arc;

use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::storage::{NotificationCategory, StorageService, Subscription};

const ADMINISTRATOR_IDS: [&str; 3] = [
    "1447045405257760820",
    "1347420765410295928",
    "1138400546823340102",
];
const USAGE_MESSAGE: &str = "使い方: o.push skd / o.push ad / o.push notice（解除は末尾に off）";
const PERMISSION_MESSAGE: &str = "通知先の変更にはBot管理者・健康維持メンテナー権限が必要です。";

pub(super) struct PushCommand {
    metadata: CommandMetadata,
    help: String,
    storage: Arc<StorageService>,
}

impl PushCommand {
    pub(super) fn new(help: &str, storage: Arc<StorageService>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "push".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            storage,
        }
    }

    async fn run(&self, context: &CommandContext, arguments: &[String]) -> CommandOutput {
        if arguments.len() == 1 && arguments[0].eq_ignore_ascii_case("help") {
            return message(context.channel_id(), &self.help);
        }
        let Some(request) = parse_request(arguments) else {
            return message(context.channel_id(), USAGE_MESSAGE);
        };
        if !ADMINISTRATOR_IDS.contains(&context.user_id()) {
            return message(context.channel_id(), PERMISSION_MESSAGE);
        }
        let Some(guild_id) = context.guild_id() else {
            return message(context.channel_id(), USAGE_MESSAGE);
        };
        let subscription = Subscription {
            guild_id: guild_id.to_owned(),
            channel_id: context.channel_id().to_owned(),
            category: request.category,
        };
        if let Err(error) = self
            .storage
            .set_subscription(subscription, request.enabled)
            .await
        {
            let content = if error.code() == "not-configured" {
                "❌ 通知先の保存先が設定されていません。"
            } else {
                eprintln!("Push subscription update failed: {error}");
                "❌ 通知先の保存に失敗しました。時間をおいて再度お試しください。"
            };
            return message(context.channel_id(), content);
        }
        message(
            context.channel_id(),
            format!(
                "{}の更新通知をこのチャンネルで{}しました。",
                request.category.label(),
                if request.enabled { "登録" } else { "解除" }
            ),
        )
    }
}

impl Command for PushCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { Ok(self.run(&context, &arguments).await) })
    }
}

struct PushRequest {
    category: NotificationCategory,
    enabled: bool,
}

fn parse_request(arguments: &[String]) -> Option<PushRequest> {
    if arguments.is_empty() || arguments.len() > 2 {
        return None;
    }
    let category = match arguments[0].to_ascii_lowercase().as_str() {
        "skd" => NotificationCategory::Skd,
        "ad" => NotificationCategory::Ad,
        "notice" => NotificationCategory::Notice,
        _ => return None,
    };
    let enabled = match arguments.get(1) {
        None => true,
        Some(value) if value.eq_ignore_ascii_case("off") => false,
        Some(_) => return None,
    };
    Some(PushRequest { category, enabled })
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}
