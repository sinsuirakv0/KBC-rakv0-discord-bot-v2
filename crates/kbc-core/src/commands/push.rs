//! 通知先をprivate GitHub Repositoryへ保存する`push` Command。

use std::sync::Arc;

use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::permissions::has_maintainer_access;
use crate::storage::{MAX_SKD_RELATED_URLS, NotificationCategory, StorageService, Subscription};

const USAGE_MESSAGE: &str = "使い方: o.push skd|ad|notice [off] / o.push skd|ad|notice role:<ロールID> [del] / o.push skd url <URL> add|del";
const PERMISSION_MESSAGE: &str = "通知先の変更にはBot管理者・Botメンテナー権限が必要です。";

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
        match has_maintainer_access(&self.storage, context).await {
            Ok(true) => {}
            Ok(false) => return message(context.channel_id(), PERMISSION_MESSAGE),
            Err(error) => {
                eprintln!("Maintainer permission load failed: {error}");
                return message(
                    context.channel_id(),
                    "❌ 権限情報の読み込みに失敗しました。時間をおいて再度お試しください。",
                );
            }
        }
        let Some(guild_id) = context.guild_id() else {
            return message(context.channel_id(), USAGE_MESSAGE);
        };
        match request {
            PushRequest::Subscription { category, enabled } => {
                let subscription = Subscription {
                    guild_id: guild_id.to_owned(),
                    channel_id: context.channel_id().to_owned(),
                    category,
                    role_ids: Vec::new(),
                };
                if let Err(error) = self.storage.set_subscription(subscription, enabled).await {
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
                        category.label(),
                        if enabled { "登録" } else { "解除" }
                    ),
                )
            }
            PushRequest::Role {
                category,
                role_id,
                enabled,
            } => match self
                .storage
                .set_subscription_role(guild_id, context.channel_id(), category, &role_id, enabled)
                .await
            {
                Ok(changed) => message(
                    context.channel_id(),
                    match (enabled, changed) {
                        (true, true) => format!(
                            "{}通知のメンションへロール `{role_id}` を追加しました。",
                            category.label()
                        ),
                        (true, false) => format!(
                            "{}通知のメンションへロール `{role_id}` は追加済みです。",
                            category.label()
                        ),
                        (false, true) => format!(
                            "{}通知のメンションからロール `{role_id}` を削除しました。",
                            category.label()
                        ),
                        (false, false) => format!(
                            "{}通知のメンションにロール `{role_id}` は登録されていません。",
                            category.label()
                        ),
                    },
                ),
                Err(error) => {
                    let content = match error.code() {
                        "notification-role-not-registered" => {
                            "❌ このロールは `o.maint pushsetting <ロールID> add` で登録されていません。"
                        }
                        "subscription-not-found" => {
                            "❌ 先にこのチャンネルを通知先として登録してください。"
                        }
                        _ => {
                            eprintln!("Push mention role update failed: {error}");
                            "❌ 通知メンション設定の保存に失敗しました。"
                        }
                    };
                    message(context.channel_id(), content)
                }
            },
            PushRequest::RelatedUrl { url, enabled } => {
                match self
                    .storage
                    .set_skd_related_url(guild_id, &url, enabled)
                    .await
                {
                    Ok(changed) => message(
                        context.channel_id(),
                        match (enabled, changed) {
                            (true, true) => "skd通知の関連サイトへURLを追加しました。",
                            (true, false) => "このURLはskd通知の関連サイトへ追加済みです。",
                            (false, true) => "skd通知の関連サイトからURLを削除しました。",
                            (false, false) => "このURLはskd通知の関連サイトに登録されていません。",
                        },
                    ),
                    Err(error) => {
                        let content = match error.code() {
                            "invalid-related-url" => {
                                "❌ `http://` または `https://` で始まる有効なURLを指定してください。".to_owned()
                            }
                            "related-url-limit-reached" => format!(
                                "❌ 関連サイトURLは最大{MAX_SKD_RELATED_URLS}件です。URLを短くする必要がある場合もあります。"
                            ),
                            _ => {
                                eprintln!("Related URL update failed: {error}");
                                "❌ 関連サイト設定の保存に失敗しました。".to_owned()
                            }
                        };
                        message(context.channel_id(), content)
                    }
                }
            }
        }
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

enum PushRequest {
    Subscription {
        category: NotificationCategory,
        enabled: bool,
    },
    Role {
        category: NotificationCategory,
        role_id: String,
        enabled: bool,
    },
    RelatedUrl {
        url: String,
        enabled: bool,
    },
}

fn parse_request(arguments: &[String]) -> Option<PushRequest> {
    if arguments.is_empty() || arguments.len() > 4 {
        return None;
    }
    let category = match arguments[0].to_ascii_lowercase().as_str() {
        "skd" => NotificationCategory::Skd,
        "ad" => NotificationCategory::Ad,
        "notice" => NotificationCategory::Notice,
        _ => return None,
    };
    if category == NotificationCategory::Skd
        && arguments.len() == 4
        && arguments[1].eq_ignore_ascii_case("url")
    {
        let enabled = if arguments[3].eq_ignore_ascii_case("add") {
            true
        } else if arguments[3].eq_ignore_ascii_case("del") {
            false
        } else {
            return None;
        };
        return Some(PushRequest::RelatedUrl {
            url: arguments[2].clone(),
            enabled,
        });
    }
    if let Some(role_id) = arguments
        .get(1)
        .and_then(|value| value.strip_prefix("role:"))
        .and_then(normalize_role_id)
    {
        let enabled = match arguments.get(2) {
            None => true,
            Some(value) if value.eq_ignore_ascii_case("del") => false,
            Some(_) => return None,
        };
        return Some(PushRequest::Role {
            category,
            role_id,
            enabled,
        });
    }
    let enabled = match arguments.get(1) {
        None => true,
        Some(value) if value.eq_ignore_ascii_case("off") && arguments.len() == 2 => false,
        Some(_) => return None,
    };
    Some(PushRequest::Subscription { category, enabled })
}

fn normalize_role_id(value: &str) -> Option<String> {
    let value = value
        .strip_prefix("<@&")
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(value);
    ((17..=20).contains(&value.len())
        && value.chars().all(|character| character.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| number > 0))
    .then(|| value.to_owned())
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}
