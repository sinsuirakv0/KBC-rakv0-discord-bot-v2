//! 通知ロールと選択パネルのDiscord操作・保存を共通化する。

use std::fmt::{Display, Formatter};
use std::sync::Arc;

use kbc_protocol::{ActionOutcome, CoreActionData, RequestId};
use tokio::sync::Mutex;

use crate::role_panel::{number_emoji, sync_action};
use crate::storage::{
    MAX_NOTIFICATION_ROLES, NotificationRole, NotificationRolePanel, StorageService,
};
use crate::task_runtime::TaskRuntime;

pub(crate) struct NotificationRoleSettingsService {
    storage: Arc<StorageService>,
    tasks: Arc<TaskRuntime>,
    panel_edit: Mutex<()>,
}

pub(crate) struct RoleUpdateResult {
    pub(crate) changed: bool,
    pub(crate) panel_updated: bool,
    pub(crate) roles_synced: bool,
}

pub(crate) struct PanelPublishResult {
    old_panel_disabled: bool,
    roles_synced: bool,
}

impl PanelPublishResult {
    pub(crate) fn message(&self, status: &str) -> String {
        let mut message = panel_update_message(status, true, self.roles_synced);
        if !self.old_panel_disabled {
            message.push_str("\n⚠️ 旧パネルを無効化できませんでした。");
        }
        message
    }
}

impl NotificationRoleSettingsService {
    pub(crate) fn new(storage: Arc<StorageService>, tasks: Arc<TaskRuntime>) -> Self {
        Self {
            storage,
            tasks,
            panel_edit: Mutex::new(()),
        }
    }

    pub(crate) async fn create_role(
        &self,
        request_id: &RequestId,
        guild_id: &str,
        name: String,
    ) -> Result<NotificationRole, NotificationRoleSettingsError> {
        match self
            .tasks
            .request_adapter(
                request_id,
                CoreActionData::CreateGuildRole {
                    guild_id: guild_id.to_owned(),
                    name,
                },
            )
            .await
        {
            Ok(ActionOutcome::RoleResolved { role_id, name }) => Ok(NotificationRole {
                role_id,
                name,
                reaction_index: None,
            }),
            Ok(ActionOutcome::Failure { code, .. }) => {
                Err(NotificationRoleSettingsError::new(code))
            }
            Ok(outcome) => {
                eprintln!("Unexpected role creation outcome: {outcome:?}");
                Err(NotificationRoleSettingsError::new("unexpected-outcome"))
            }
            Err(error) => {
                eprintln!("Role creation failed: {error}");
                Err(NotificationRoleSettingsError::new("adapter-action-failed"))
            }
        }
    }

    pub(crate) async fn set_role(
        &self,
        request_id: &RequestId,
        guild_id: &str,
        role_id: &str,
        enabled: bool,
    ) -> Result<RoleUpdateResult, NotificationRoleSettingsError> {
        let _edit = self.panel_edit.lock().await;
        let previous = self
            .storage
            .notification_role_settings(guild_id)
            .await
            .map_err(|error| NotificationRoleSettingsError::new(error.code()))?;
        let current_index = previous
            .roles
            .iter()
            .find(|role| role.role_id == role_id)
            .and_then(|role| role.reaction_index);
        let mut role = if enabled {
            self.resolve_role(request_id, guild_id, role_id).await?
        } else {
            NotificationRole {
                role_id: role_id.to_owned(),
                name: "削除対象".to_owned(),
                reaction_index: current_index,
            }
        };
        if enabled {
            let index = current_index
                .or_else(|| {
                    (0..MAX_NOTIFICATION_ROLES).find(|index| {
                        previous
                            .roles
                            .iter()
                            .all(|role| role.reaction_index != Some(*index))
                    })
                })
                .ok_or_else(|| {
                    NotificationRoleSettingsError::new("notification-role-limit-reached")
                })?;
            role.reaction_index = Some(index);
            if current_index.is_none()
                && let Some(panel) = &previous.panel
            {
                self.update_panel_reaction(request_id, guild_id, panel, index, true)
                    .await?;
            }
        }
        let changed = self
            .storage
            .set_notification_role(guild_id, &role, enabled)
            .await
            .map_err(|error| NotificationRoleSettingsError::new(error.code()))?;
        let settings = self
            .storage
            .notification_role_settings(guild_id)
            .await
            .map_err(|error| NotificationRoleSettingsError::new(error.code()))?;
        let (panel_updated, roles_synced) = match &settings.panel {
            Some(panel) => {
                let edited = self.refresh_panel(request_id, panel, &settings.roles).await;
                let mut cleaned = true;
                if !enabled && let Some(index) = current_index {
                    cleaned = self
                        .update_panel_reaction(request_id, guild_id, panel, index, false)
                        .await
                        .is_ok();
                }
                let synced = edited
                    && self
                        .sync_panel(request_id, guild_id, panel, &settings.roles)
                        .await;
                (edited && cleaned, synced)
            }
            None => (true, true),
        };
        Ok(RoleUpdateResult {
            changed,
            panel_updated,
            roles_synced,
        })
    }

    pub(crate) async fn publish_panel(
        &self,
        request_id: &RequestId,
        guild_id: &str,
        channel_id: &str,
    ) -> Result<PanelPublishResult, NotificationRoleSettingsError> {
        let _edit = self.panel_edit.lock().await;
        let settings = self
            .storage
            .notification_role_settings(guild_id)
            .await
            .map_err(|error| NotificationRoleSettingsError::new(error.code()))?;
        let outcome = self
            .tasks
            .request_adapter(
                request_id,
                CoreActionData::SendMessage {
                    channel_id: channel_id.to_owned(),
                    content: panel_content(&settings.roles),
                },
            )
            .await;
        let Some(message_id) = successful_message_id(outcome) else {
            return Err(NotificationRoleSettingsError::new("panel-send-failed"));
        };
        let new_panel = NotificationRolePanel {
            channel_id: channel_id.to_owned(),
            message_id,
        };
        if !self
            .add_panel_reactions(request_id, &new_panel, &settings.roles)
            .await
        {
            let _ = self
                .disable_panel(request_id, &new_panel, "通知設定の作成に失敗しました")
                .await;
            return Err(NotificationRoleSettingsError::new("panel-reaction-failed"));
        }
        if let Err(error) = self
            .storage
            .set_notification_role_panel(guild_id, new_panel.clone())
            .await
        {
            eprintln!("Notification role panel save failed: {error}");
            let _ = self
                .disable_panel(request_id, &new_panel, "通知設定の保存に失敗しました")
                .await;
            return Err(NotificationRoleSettingsError::new(error.code()));
        }
        let old_panel_disabled = match settings.panel {
            Some(old_panel) if old_panel != new_panel => {
                self.disable_panel(request_id, &old_panel, "通知設定は移動しました")
                    .await
            }
            _ => true,
        };
        let roles_synced = self
            .sync_panel(request_id, guild_id, &new_panel, &settings.roles)
            .await;
        Ok(PanelPublishResult {
            old_panel_disabled,
            roles_synced,
        })
    }

    pub(crate) async fn sync_saved_panels(&self, request_id: &RequestId) {
        let guild_ids = match self.storage.notification_role_panel_guilds().await {
            Ok(guild_ids) => guild_ids,
            Err(error) => {
                eprintln!("Notification role panels could not be loaded: {error}");
                return;
            }
        };
        for guild_id in guild_ids {
            let _edit = self.panel_edit.lock().await;
            match self.storage.notification_role_settings(&guild_id).await {
                Ok(settings) => {
                    if let Some(panel) = settings.panel {
                        self.sync_panel(request_id, &guild_id, &panel, &settings.roles)
                            .await;
                    }
                }
                Err(error) => eprintln!("Notification role panel lookup failed: {error}"),
            }
        }
    }

    async fn resolve_role(
        &self,
        request_id: &RequestId,
        guild_id: &str,
        role_id: &str,
    ) -> Result<NotificationRole, NotificationRoleSettingsError> {
        match self
            .tasks
            .request_adapter(
                request_id,
                CoreActionData::ResolveAssignableRole {
                    guild_id: guild_id.to_owned(),
                    role_id: role_id.to_owned(),
                },
            )
            .await
        {
            Ok(ActionOutcome::RoleResolved { role_id, name }) => Ok(NotificationRole {
                role_id,
                name,
                reaction_index: None,
            }),
            Ok(ActionOutcome::Failure { code, .. }) => {
                Err(NotificationRoleSettingsError::new(code))
            }
            Ok(outcome) => {
                eprintln!("Unexpected role resolution outcome: {outcome:?}");
                Err(NotificationRoleSettingsError::new("unexpected-outcome"))
            }
            Err(error) => {
                eprintln!("Role resolution failed: {error}");
                Err(NotificationRoleSettingsError::new("adapter-action-failed"))
            }
        }
    }

    async fn refresh_panel(
        &self,
        request_id: &RequestId,
        panel: &NotificationRolePanel,
        roles: &[NotificationRole],
    ) -> bool {
        self.request_success(
            request_id,
            CoreActionData::EditMessage {
                channel_id: panel.channel_id.clone(),
                message_id: panel.message_id.clone(),
                content: panel_content(roles),
            },
        )
        .await
    }

    async fn sync_panel(
        &self,
        request_id: &RequestId,
        guild_id: &str,
        panel: &NotificationRolePanel,
        roles: &[NotificationRole],
    ) -> bool {
        let Some(action) = sync_action(guild_id, panel, roles, None, None) else {
            return true;
        };
        self.request_success(request_id, action).await
    }

    async fn update_panel_reaction(
        &self,
        request_id: &RequestId,
        guild_id: &str,
        panel: &NotificationRolePanel,
        index: usize,
        enabled: bool,
    ) -> Result<(), NotificationRoleSettingsError> {
        let Some(emoji) = number_emoji(index) else {
            return Err(NotificationRoleSettingsError::new(
                "invalid-notification-role-slot",
            ));
        };
        match self
            .tasks
            .request_adapter(
                request_id,
                CoreActionData::UpdatePanelReaction {
                    guild_id: guild_id.to_owned(),
                    channel_id: panel.channel_id.clone(),
                    message_id: panel.message_id.clone(),
                    emoji: emoji.to_owned(),
                    enabled,
                },
            )
            .await
        {
            Ok(ActionOutcome::Success { .. }) => Ok(()),
            Ok(ActionOutcome::Failure { code, .. }) => {
                Err(NotificationRoleSettingsError::new(code))
            }
            _ => Err(NotificationRoleSettingsError::new("panel-reaction-failed")),
        }
    }

    async fn add_panel_reactions(
        &self,
        request_id: &RequestId,
        panel: &NotificationRolePanel,
        roles: &[NotificationRole],
    ) -> bool {
        for role in roles {
            let Some(emoji) = role.reaction_index.and_then(number_emoji) else {
                return false;
            };
            if !self
                .request_success(
                    request_id,
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
        request_id: &RequestId,
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
            if !self.request_success(request_id, action).await {
                return false;
            }
        }
        true
    }

    async fn request_success(&self, request_id: &RequestId, action: CoreActionData) -> bool {
        matches!(
            self.tasks.request_adapter(request_id, action).await,
            Ok(ActionOutcome::Success { .. })
        )
    }
}

fn panel_content(roles: &[NotificationRole]) -> String {
    let mut lines = vec!["通知設定".to_owned()];
    if roles.is_empty() {
        lines.push("設定されている通知ロールはありません".to_owned());
    } else {
        let mut ordered: Vec<_> = roles.iter().collect();
        ordered.sort_unstable_by_key(|role| role.reaction_index);
        lines.extend(ordered.into_iter().filter_map(|role| {
            let index = role.reaction_index?;
            Some(format!("{} {}", index + 1, role.name.replace('`', "′")))
        }));
    }
    format!("```\n{}\n```", lines.join("\n"))
}

pub(crate) fn panel_update_message(
    status: &str,
    panel_updated: bool,
    roles_synced: bool,
) -> String {
    let mut message = status.to_owned();
    if !panel_updated {
        message.push_str("\n⚠️ ロール選択パネルの表示またはリアクションを更新できませんでした。");
    }
    if !roles_synced {
        message.push_str("\n⚠️ ロールとリアクションの同期を完了できませんでした。");
    }
    message
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

#[derive(Debug)]
pub(crate) struct NotificationRoleSettingsError {
    code: String,
}

impl NotificationRoleSettingsError {
    fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }
}

impl Display for NotificationRoleSettingsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.code)
    }
}
