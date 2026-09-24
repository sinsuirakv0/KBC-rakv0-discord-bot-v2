//! 全サーバー共通のBotメンテナー設定を管理する`maint` Command。

use std::sync::Arc;

use kbc_protocol::{ActionOutcome, CoreActionData};

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::permissions::{has_maintainer_access, is_bot_administrator};
use crate::storage::{MAX_MAINTAINER_SUBJECTS, StorageService};
use crate::task_runtime::TaskRuntime;

const USAGE_MESSAGE: &str =
    "使い方: o.maint maintainer <ユーザーIDまたはロールID> [del] / o.maint maintainer list";
const UPDATE_PERMISSION_MESSAGE: &str = "メンテナーの変更は固定Bot管理者だけが実行できます。";
const LIST_PERMISSION_MESSAGE: &str =
    "メンテナー一覧は固定Bot管理者またはBotメンテナーだけが表示できます。";
const MAX_LIST_MEMBERS: u16 = 25;

pub(super) struct MaintCommand {
    metadata: CommandMetadata,
    help: String,
    storage: Arc<StorageService>,
    tasks: Arc<TaskRuntime>,
}

impl MaintCommand {
    pub(super) fn new(help: &str, storage: Arc<StorageService>, tasks: Arc<TaskRuntime>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "maint".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            storage,
            tasks,
        }
    }

    async fn run(&self, context: &CommandContext, arguments: &[String]) -> CommandOutput {
        if arguments.len() == 1 && arguments[0].eq_ignore_ascii_case("help") {
            return message(context.channel_id(), &self.help);
        }
        let Some(request) = parse_request(arguments) else {
            return message(context.channel_id(), USAGE_MESSAGE);
        };
        match request {
            MaintRequest::Update {
                subject_id,
                enabled,
            } => {
                if !is_bot_administrator(context.user_id()) {
                    return message(context.channel_id(), UPDATE_PERMISSION_MESSAGE);
                }
                self.update(context, &subject_id, enabled).await
            }
            MaintRequest::List => match has_maintainer_access(&self.storage, context).await {
                Ok(true) => self.list(context).await,
                Ok(false) => message(context.channel_id(), LIST_PERMISSION_MESSAGE),
                Err(error) => {
                    eprintln!("Maintainer permission load failed: {error}");
                    message(
                        context.channel_id(),
                        "❌ 権限情報の読み込みに失敗しました。時間をおいて再度お試しください。",
                    )
                }
            },
        }
    }

    async fn update(
        &self,
        context: &CommandContext,
        subject_id: &str,
        enabled: bool,
    ) -> CommandOutput {
        match self.storage.set_maintainer(subject_id, enabled).await {
            Ok(changed) => message(
                context.channel_id(),
                match (enabled, changed) {
                    (true, true) => format!("メンテナー対象 `{subject_id}` を追加しました。"),
                    (true, false) => format!("メンテナー対象 `{subject_id}` は追加済みです。"),
                    (false, true) => format!("メンテナー対象 `{subject_id}` を削除しました。"),
                    (false, false) => {
                        format!("メンテナー対象 `{subject_id}` は登録されていません。")
                    }
                },
            ),
            Err(error) => {
                let content = match error.code() {
                    "not-configured" => "❌ メンテナー設定の保存先が設定されていません。",
                    "maintainer-limit-reached" => {
                        return message(
                            context.channel_id(),
                            format!("❌ メンテナー対象は最大{MAX_MAINTAINER_SUBJECTS}件までです。"),
                        );
                    }
                    _ => {
                        eprintln!("Maintainer update failed: {error}");
                        "❌ メンテナー設定の保存に失敗しました。時間をおいて再度お試しください。"
                    }
                };
                message(context.channel_id(), content)
            }
        }
    }

    async fn list(&self, context: &CommandContext) -> CommandOutput {
        let subject_ids = match self.storage.maintainer_subject_ids().await {
            Ok(subject_ids) => subject_ids,
            Err(error) => {
                eprintln!("Maintainer list load failed: {error}");
                return message(
                    context.channel_id(),
                    "❌ メンテナー設定の読み込みに失敗しました。",
                );
            }
        };
        if subject_ids.is_empty() {
            return message(context.channel_id(), "登録されているメンテナーはいません。");
        }
        let Some(guild_id) = context.guild_id() else {
            return message(context.channel_id(), USAGE_MESSAGE);
        };
        let outcome = self
            .tasks
            .request_adapter(
                context.request_id(),
                CoreActionData::ResolveGuildMembers {
                    guild_id: guild_id.to_owned(),
                    subject_ids,
                    max_members: MAX_LIST_MEMBERS,
                },
            )
            .await;
        match outcome {
            Ok(ActionOutcome::MembersResolved { members, .. }) if members.is_empty() => message(
                context.channel_id(),
                "このサーバー内に該当するメンテナーはいません。",
            ),
            Ok(ActionOutcome::MembersResolved { members, truncated }) => {
                let mut lines = members
                    .into_iter()
                    .map(|member| format!("- {} (`{}`)", member.display_name, member.user_id))
                    .collect::<Vec<_>>();
                if truncated {
                    lines.push(format!("- ほか（最大{MAX_LIST_MEMBERS}人まで表示）"));
                }
                message(
                    context.channel_id(),
                    format!("このサーバーのメンテナー:\n{}", lines.join("\n")),
                )
            }
            Ok(outcome) => {
                eprintln!("Unexpected maintainer resolution outcome: {outcome:?}");
                message(
                    context.channel_id(),
                    "❌ メンテナー一覧を取得できませんでした。",
                )
            }
            Err(error) => {
                eprintln!("Maintainer resolution failed: {error}");
                message(
                    context.channel_id(),
                    "❌ メンテナー一覧を取得できませんでした。",
                )
            }
        }
    }
}

impl Command for MaintCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { Ok(self.run(&context, &arguments).await) })
    }
}

enum MaintRequest {
    Update { subject_id: String, enabled: bool },
    List,
}

fn parse_request(arguments: &[String]) -> Option<MaintRequest> {
    if !arguments.first()?.eq_ignore_ascii_case("maintainer") {
        return None;
    }
    if arguments.len() == 2 && arguments[1].eq_ignore_ascii_case("list") {
        return Some(MaintRequest::List);
    }
    if !(2..=3).contains(&arguments.len()) {
        return None;
    }
    let subject_id = normalize_subject_id(&arguments[1])?;
    let enabled = match arguments.get(2) {
        None => true,
        Some(value) if value.eq_ignore_ascii_case("del") => false,
        Some(_) => return None,
    };
    Some(MaintRequest::Update {
        subject_id,
        enabled,
    })
}

fn normalize_subject_id(value: &str) -> Option<String> {
    let unwrapped = value
        .strip_prefix("<@&")
        .or_else(|| value.strip_prefix("<@!"))
        .or_else(|| value.strip_prefix("<@"))
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(value);
    ((17..=20).contains(&unwrapped.len())
        && unwrapped
            .chars()
            .all(|character| character.is_ascii_digit())
        && unwrapped.parse::<u64>().is_ok_and(|number| number > 0))
    .then(|| unwrapped.to_owned())
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_user_and_role_mentions() {
        assert_eq!(
            normalize_subject_id("<@123456789012345678>"),
            Some("123456789012345678".to_owned())
        );
        assert_eq!(
            normalize_subject_id("<@&987654321098765432>"),
            Some("987654321098765432".to_owned())
        );
        assert_eq!(normalize_subject_id("00000000000000000"), None);
    }
}
