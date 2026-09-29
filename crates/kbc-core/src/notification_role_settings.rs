//! 通知ロールと選択パネルのDiscord操作・保存を共通化する。

use std::fmt::{Display, Formatter};
use std::sync::Arc;

use kbc_protocol::{ActionOutcome, CoreActionData, RequestId};

use crate::role_panel::number_emoji;
use crate::storage::{NotificationRole, NotificationRolePanel, StorageService};
use crate::task_runtime::TaskRuntime;

pub(crate) struct NotificationRoleSettingsService {
    storage: Arc<StorageService>,
    tasks: Arc<TaskRuntime>,
}

pub(crate) struct RoleUpdateResult {
    pub(crate) changed: bool,
    pub(crate) panel_updated: bool,
}

pub(crate) struct PanelPublishResult {
    pub(crate) old_panel_disabled: bool,
}

impl NotificationRoleSettingsService {
    pub(crate) fn new(storage: Arc<StorageService>, tasks: Arc<TaskRuntime>) -> Self {
        Self { storage, tasks }
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
            Ok(ActionOutcome::RoleResolved { role_id, name }) => {
                Ok(NotificationRole { role_id, name })
            }
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
        let role = if enabled {
            self.resolve_role(request_id, guild_id, role_id).await?
        } else {
            NotificationRole {
                role_id: role_id.to_owned(),
                name: "削除対象".to_owned(),
            }
        };
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
        let panel_updated = match &settings.panel {
            Some(panel) => self.refresh_panel(request_id, panel, &settings.roles).await,
            None => true,
        };
        Ok(RoleUpdateResult {
            changed,
            panel_updated,
        })
    }

    pub(crate) async fn publish_panel(
        &self,
        request_id: &RequestId,
        guild_id: &str,
        channel_id: &str,
    ) -> Result<PanelPublishResult, NotificationRoleSettingsError> {
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
            .add_panel_reactions(request_id, &new_panel, settings.roles.len())
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
        Ok(PanelPublishResult { old_panel_disabled })
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
            Ok(ActionOutcome::RoleResolved { role_id, name }) => {
                Ok(NotificationRole { role_id, name })
            }
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
            if !self.request_success(request_id, action).await {
                return false;
            }
        }
        self.add_panel_reactions(request_id, panel, roles.len())
            .await
    }

    async fn add_panel_reactions(
        &self,
        request_id: &RequestId,
        panel: &NotificationRolePanel,
        count: usize,
    ) -> bool {
        for index in 0..count {
            let Some(emoji) = number_emoji(index) else {
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
