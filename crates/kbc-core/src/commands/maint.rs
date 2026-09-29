//! Botメンテナーと通知用ロールの設定を管理する`maint` Command。

use std::sync::Arc;

use kbc_protocol::{ActionOutcome, CoreActionData};

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::notification_role_settings::NotificationRoleSettingsService;
use crate::permissions::{has_maintainer_access, is_bot_administrator};
use crate::storage::{MAX_MAINTAINER_SUBJECTS, MAX_NOTIFICATION_ROLES, StorageService};
use crate::task_runtime::TaskRuntime;

const USAGE_MESSAGE: &str = "使い方: o.maint maintainer <ID> [del] / o.maint maintainer list / o.maint role <ロール名> / o.maint pushsetting [ロールID add|del]";
const UPDATE_PERMISSION_MESSAGE: &str = "メンテナーの変更は固定Bot管理者だけが実行できます。";
const MAINTAINER_PERMISSION_MESSAGE: &str = "この操作にはBot管理者・Botメンテナー権限が必要です。";
const MAX_LIST_MEMBERS: u16 = 25;

pub(super) struct MaintCommand {
    metadata: CommandMetadata,
    help: String,
    storage: Arc<StorageService>,
    tasks: Arc<TaskRuntime>,
    role_settings: Arc<NotificationRoleSettingsService>,
}

impl MaintCommand {
    pub(super) fn new(
        help: &str,
        storage: Arc<StorageService>,
        tasks: Arc<TaskRuntime>,
        role_settings: Arc<NotificationRoleSettingsService>,
    ) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "maint".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            storage,
            tasks,
            role_settings,
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
            MaintRequest::MaintainerUpdate {
                subject_id,
                enabled,
            } => {
                if !is_bot_administrator(context.user_id()) {
                    return message(context.channel_id(), UPDATE_PERMISSION_MESSAGE);
                }
                self.update_maintainer(context, &subject_id, enabled).await
            }
            MaintRequest::MaintainerList => {
                if let Some(output) = self.require_maintainer(context).await {
                    return output;
                }
                self.list_maintainers(context).await
            }
            MaintRequest::CreateRole { name } => {
                if let Some(output) = self.require_maintainer(context).await {
                    return output;
                }
                self.create_role(context, name).await
            }
            MaintRequest::PublishRolePanel => {
                if let Some(output) = self.require_maintainer(context).await {
                    return output;
                }
                self.publish_role_panel(context).await
            }
            MaintRequest::UpdatePanelRole { role_id, enabled } => {
                if let Some(output) = self.require_maintainer(context).await {
                    return output;
                }
                self.update_panel_role(context, &role_id, enabled).await
            }
        }
    }

    async fn require_maintainer(&self, context: &CommandContext) -> Option<CommandOutput> {
        match has_maintainer_access(&self.storage, context).await {
            Ok(true) => None,
            Ok(false) => Some(message(context.channel_id(), MAINTAINER_PERMISSION_MESSAGE)),
            Err(error) => {
                eprintln!("Maintainer permission load failed: {error}");
                Some(message(
                    context.channel_id(),
                    "❌ 権限情報の読み込みに失敗しました。時間をおいて再度お試しください。",
                ))
            }
        }
    }

    async fn update_maintainer(
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

    async fn list_maintainers(&self, context: &CommandContext) -> CommandOutput {
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

    async fn create_role(&self, context: &CommandContext, name: String) -> CommandOutput {
        let guild_id = context.guild_id().expect("guild-only command has a guild");
        match self
            .role_settings
            .create_role(context.request_id(), guild_id, name)
            .await
        {
            Ok(role) => message(
                context.channel_id(),
                format!(
                    "通知用ロール `@{}` (`{}`) を作成しました。",
                    role.name, role.role_id
                ),
            ),
            Err(error) => {
                eprintln!("Role creation failed: {error}");
                message(context.channel_id(), "❌ ロールを作成できませんでした。")
            }
        }
    }

    async fn update_panel_role(
        &self,
        context: &CommandContext,
        role_id: &str,
        enabled: bool,
    ) -> CommandOutput {
        let guild_id = context.guild_id().expect("guild-only command has a guild");
        let result = match self
            .role_settings
            .set_role(context.request_id(), guild_id, role_id, enabled)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let content = match error.code() {
                    "role_has_permissions" | "role_not_manageable" | "role_unavailable" => {
                        role_validation_message(error.code()).to_owned()
                    }
                    "notification-role-limit-reached" => {
                        format!("❌ 通知ロールは最大{MAX_NOTIFICATION_ROLES}件までです。")
                    }
                    _ => {
                        eprintln!("Notification role update failed: {error}");
                        "❌ 通知ロール設定の保存に失敗しました。".to_owned()
                    }
                };
                return message(context.channel_id(), content);
            }
        };
        let status = match (enabled, result.changed) {
            (true, true) => "通知設定へ追加しました。",
            (true, false) => "通知設定へ追加済みです。",
            (false, true) => "通知設定から削除しました。関連する通知メンションも解除しました。",
            (false, false) => "通知設定には登録されていません。",
        };
        message(
            context.channel_id(),
            if result.panel_updated {
                status.to_owned()
            } else {
                format!(
                    "{status}\n⚠️ 選択メッセージの更新に失敗しました。`o.maint pushsetting` を再実行してください。"
                )
            },
        )
    }

    async fn publish_role_panel(&self, context: &CommandContext) -> CommandOutput {
        let guild_id = context.guild_id().expect("guild-only command has a guild");
        match self
            .role_settings
            .publish_panel(context.request_id(), guild_id, context.channel_id())
            .await
        {
            Ok(result) => message(
                context.channel_id(),
                if result.old_panel_disabled {
                    "通知設定メッセージを設置しました。"
                } else {
                    "通知設定メッセージを設置しました。旧メッセージの無効化だけ失敗しました。"
                },
            ),
            Err(error) => {
                eprintln!("Notification role panel publish failed: {error}");
                message(
                    context.channel_id(),
                    "❌ 通知設定メッセージを作成できませんでした。",
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
    MaintainerUpdate { subject_id: String, enabled: bool },
    MaintainerList,
    CreateRole { name: String },
    PublishRolePanel,
    UpdatePanelRole { role_id: String, enabled: bool },
}

fn parse_request(arguments: &[String]) -> Option<MaintRequest> {
    match arguments.first()?.to_ascii_lowercase().as_str() {
        "maintainer" => parse_maintainer_request(arguments),
        "role" if arguments.len() >= 2 => {
            let name = arguments[1..].join(" ");
            (!name.is_empty() && name.chars().count() <= 100)
                .then_some(MaintRequest::CreateRole { name })
        }
        "pushsetting" if arguments.len() == 1 => Some(MaintRequest::PublishRolePanel),
        "pushsetting" if arguments.len() == 3 => {
            let role_id = normalize_subject_id(&arguments[1])?;
            let enabled = if arguments[2].eq_ignore_ascii_case("add") {
                true
            } else if arguments[2].eq_ignore_ascii_case("del") {
                false
            } else {
                return None;
            };
            Some(MaintRequest::UpdatePanelRole { role_id, enabled })
        }
        _ => None,
    }
}

fn parse_maintainer_request(arguments: &[String]) -> Option<MaintRequest> {
    if arguments.len() == 2 && arguments[1].eq_ignore_ascii_case("list") {
        return Some(MaintRequest::MaintainerList);
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
    Some(MaintRequest::MaintainerUpdate {
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

fn role_validation_message(code: &str) -> &'static str {
    match code {
        "role_has_permissions" => "❌ 権限を持つロールは通知設定へ追加できません。",
        "role_not_manageable" => "❌ Botが管理できないロールは通知設定へ追加できません。",
        "role_unavailable" => "❌ 指定したロールが見つかりません。",
        _ => "❌ ロールを確認できませんでした。",
    }
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
