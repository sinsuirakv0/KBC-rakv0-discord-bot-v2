//! Botメンテナーと通知用ロールの設定を管理する`maint` Command。

use std::sync::Arc;

use kbc_protocol::{ActionOutcome, CoreActionData};

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::permissions::{has_maintainer_access, is_bot_administrator};
use crate::role_panel::number_emoji;
use crate::storage::{
    MAX_MAINTAINER_SUBJECTS, MAX_NOTIFICATION_ROLES, NotificationRole, NotificationRolePanel,
    StorageService,
};
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
            .tasks
            .request_adapter(
                context.request_id(),
                CoreActionData::CreateGuildRole {
                    guild_id: guild_id.to_owned(),
                    name,
                },
            )
            .await
        {
            Ok(ActionOutcome::RoleResolved { role_id, name }) => message(
                context.channel_id(),
                format!("通知用ロール `@{name}` (`{role_id}`) を作成しました。"),
            ),
            Ok(outcome) => {
                eprintln!("Unexpected role creation outcome: {outcome:?}");
                message(context.channel_id(), "❌ ロールを作成できませんでした。")
            }
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
        let role = if enabled {
            match self
                .tasks
                .request_adapter(
                    context.request_id(),
                    CoreActionData::ResolveAssignableRole {
                        guild_id: guild_id.to_owned(),
                        role_id: role_id.to_owned(),
                    },
                )
                .await
            {
                Ok(ActionOutcome::RoleResolved { role_id, name }) => {
                    NotificationRole { role_id, name }
                }
                Ok(ActionOutcome::Failure { code, .. }) => {
                    return message(context.channel_id(), role_validation_message(&code));
                }
                Ok(outcome) => {
                    eprintln!("Unexpected role resolution outcome: {outcome:?}");
                    return message(context.channel_id(), "❌ ロールを確認できませんでした。");
                }
                Err(error) => {
                    eprintln!("Role resolution failed: {error}");
                    return message(context.channel_id(), "❌ ロールを確認できませんでした。");
                }
            }
        } else {
            NotificationRole {
                role_id: role_id.to_owned(),
                name: "削除対象".to_owned(),
            }
        };
        let changed = match self
            .storage
            .set_notification_role(guild_id, &role, enabled)
            .await
        {
            Ok(changed) => changed,
            Err(error) => {
                let content = if error.code() == "notification-role-limit-reached" {
                    format!("❌ 通知ロールは最大{MAX_NOTIFICATION_ROLES}件までです。")
                } else {
                    eprintln!("Notification role update failed: {error}");
                    "❌ 通知ロール設定の保存に失敗しました。".to_owned()
                };
                return message(context.channel_id(), content);
            }
        };
        let settings = match self.storage.notification_role_settings(guild_id).await {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!("Notification role settings reload failed: {error}");
                return message(
                    context.channel_id(),
                    "❌ 通知ロール設定を再読込できませんでした。",
                );
            }
        };
        let panel_updated = match &settings.panel {
            Some(panel) => self.refresh_panel(context, panel, &settings.roles).await,
            None => true,
        };
        let status = match (enabled, changed) {
            (true, true) => "通知設定へ追加しました。",
            (true, false) => "通知設定へ追加済みです。",
            (false, true) => "通知設定から削除しました。関連する通知メンションも解除しました。",
            (false, false) => "通知設定には登録されていません。",
        };
        message(
            context.channel_id(),
            if panel_updated {
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
        let settings = match self.storage.notification_role_settings(guild_id).await {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!("Notification role settings load failed: {error}");
                return message(
                    context.channel_id(),
                    "❌ 通知ロール設定を読み込めませんでした。",
                );
            }
        };
        let outcome = self
            .tasks
            .request_adapter(
                context.request_id(),
                CoreActionData::SendMessage {
                    channel_id: context.channel_id().to_owned(),
                    content: panel_content(&settings.roles),
                },
            )
            .await;
        let Some(message_id) = successful_message_id(outcome) else {
            return message(
                context.channel_id(),
                "❌ 通知設定メッセージを作成できませんでした。",
            );
        };
        let new_panel = NotificationRolePanel {
            channel_id: context.channel_id().to_owned(),
            message_id,
        };
        if !self
            .add_panel_reactions(context, &new_panel, settings.roles.len())
            .await
        {
            let _ = self
                .disable_panel(context, &new_panel, "通知設定の作成に失敗しました")
                .await;
            return message(
                context.channel_id(),
                "❌ 通知設定のリアクションを追加できませんでした。",
            );
        }
        if let Err(error) = self
            .storage
            .set_notification_role_panel(guild_id, new_panel.clone())
            .await
        {
            eprintln!("Notification role panel save failed: {error}");
            let _ = self
                .disable_panel(context, &new_panel, "通知設定の保存に失敗しました")
                .await;
            return message(
                context.channel_id(),
                "❌ 通知設定メッセージを保存できませんでした。",
            );
        }
        let old_panel_disabled = match settings.panel {
            Some(old_panel) if old_panel != new_panel => {
                self.disable_panel(context, &old_panel, "通知設定は移動しました")
                    .await
            }
            _ => true,
        };
        message(
            context.channel_id(),
            if old_panel_disabled {
                "通知設定メッセージを設置しました。"
            } else {
                "通知設定メッセージを設置しました。旧メッセージの無効化だけ失敗しました。"
            },
        )
    }

    async fn refresh_panel(
        &self,
        context: &CommandContext,
        panel: &NotificationRolePanel,
        roles: &[NotificationRole],
    ) -> bool {
        for action in [
            CoreActionData::EditMessage {
                channel_id: panel.channel_id.clone(),
                message_id: panel.message_id.clone(),
                content: panel_content(roles),
            },
            CoreActionData::ClearReactions {
                channel_id: panel.channel_id.clone(),
                message_id: panel.message_id.clone(),
            },
        ] {
            if !self.request_success(context, action).await {
                return false;
            }
        }
        self.add_panel_reactions(context, panel, roles.len()).await
    }

    async fn add_panel_reactions(
        &self,
        context: &CommandContext,
        panel: &NotificationRolePanel,
        count: usize,
    ) -> bool {
        for index in 0..count {
            let Some(emoji) = number_emoji(index) else {
                return false;
            };
            if !self
                .request_success(
                    context,
                    CoreActionData::AddReaction {
                        channel_id: panel.channel_id.clone(),
                        message_id: panel.message_id.clone(),
                        emoji: emoji.to_owned(),
                    },
                )
                .await
            {
                return false;
            }
        }
        true
    }

    async fn disable_panel(
        &self,
        context: &CommandContext,
        panel: &NotificationRolePanel,
        content: &str,
    ) -> bool {
        for action in [
            CoreActionData::ClearReactions {
                channel_id: panel.channel_id.clone(),
                message_id: panel.message_id.clone(),
            },
            CoreActionData::EditMessage {
                channel_id: panel.channel_id.clone(),
                message_id: panel.message_id.clone(),
                content: content.to_owned(),
            },
        ] {
            if !self.request_success(context, action).await {
                return false;
            }
        }
        true
    }

    async fn request_success(&self, context: &CommandContext, action: CoreActionData) -> bool {
        matches!(
            self.tasks
                .request_adapter(context.request_id(), action)
                .await,
            Ok(ActionOutcome::Success { .. })
        )
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

fn panel_content(roles: &[NotificationRole]) -> String {
    let mut lines = vec!["通知設定".to_owned()];
    if roles.is_empty() {
        lines.push("設定されている通知ロールはありません".to_owned());
    } else {
        lines.extend(
            roles
                .iter()
                .enumerate()
                .map(|(index, role)| format!("{} {}", index + 1, role.name.replace('`', "′"))),
        );
    }
    format!("```\n{}\n```", lines.join("\n"))
}

fn successful_message_id(outcome: Result<ActionOutcome, &'static str>) -> Option<String> {
    match outcome {
        Ok(ActionOutcome::Success {
            message_id: Some(message_id),
        }) => Some(message_id),
        Ok(outcome) => {
            eprintln!("Unexpected panel message outcome: {outcome:?}");
            None
        }
        Err(error) => {
            eprintln!("Panel message action failed: {error}");
            None
        }
    }
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
