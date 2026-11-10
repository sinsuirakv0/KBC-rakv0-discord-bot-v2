//! 永続的な通知ロール選択パネルのReactionをRole操作へ変換する。

use std::sync::Arc;

use kbc_protocol::{CoreActionData, CoreEventData, ReactionRoleBinding};

use crate::storage::{NotificationRole, NotificationRolePanel, StorageService};

const NUMBER_EMOJIS: [&str; 9] = ["1️⃣", "2️⃣", "3️⃣", "4️⃣", "5️⃣", "6️⃣", "7️⃣", "8️⃣", "9️⃣"];

pub(crate) struct RolePanelService {
    storage: Arc<StorageService>,
}

impl RolePanelService {
    pub(crate) fn new(storage: Arc<StorageService>) -> Self {
        Self { storage }
    }

    pub(crate) async fn action_for_event(&self, event: &CoreEventData) -> Option<CoreActionData> {
        let (guild_id, channel_id, message_id, emoji, user_id) = match event {
            CoreEventData::ReactionAdd {
                guild_id,
                channel_id,
                message_id,
                user_id,
                emoji,
            }
            | CoreEventData::ReactionRemove {
                guild_id,
                channel_id,
                message_id,
                user_id,
                emoji,
            } => (
                guild_id.as_ref()?,
                channel_id,
                message_id,
                Some(emoji.as_str()),
                Some(user_id.as_str()),
            ),
            CoreEventData::ReactionClear {
                guild_id,
                channel_id,
                message_id,
                emoji,
            } => (guild_id, channel_id, message_id, emoji.as_deref(), None),
            _ => return None,
        };
        let index = match emoji {
            Some(emoji) => Some(reaction_index(emoji)?),
            None => None,
        };
        let settings = match self.storage.notification_role_settings(guild_id).await {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!("Notification role lookup failed: {error}");
                return None;
            }
        };
        let panel = settings.panel?;
        if panel.channel_id != *channel_id || panel.message_id != *message_id {
            return None;
        }
        sync_action(guild_id, &panel, &settings.roles, index, user_id)
    }
}

pub(crate) fn number_emoji(index: usize) -> Option<&'static str> {
    NUMBER_EMOJIS.get(index).copied()
}

fn reaction_index(emoji: &str) -> Option<usize> {
    NUMBER_EMOJIS.iter().position(|number| {
        *number == emoji
            || number
                .chars()
                .filter(|character| *character != '\u{fe0f}')
                .eq(emoji.chars())
    })
}

pub(crate) fn sync_action(
    guild_id: &str,
    panel: &NotificationRolePanel,
    roles: &[NotificationRole],
    index_filter: Option<usize>,
    user_id: Option<&str>,
) -> Option<CoreActionData> {
    let bindings: Vec<_> = roles
        .iter()
        .filter_map(|role| {
            let index = role.reaction_index?;
            if index_filter.is_some_and(|filter| filter != index) {
                return None;
            }
            Some(ReactionRoleBinding {
                emoji: number_emoji(index)?.to_owned(),
                role_id: role.role_id.clone(),
            })
        })
        .collect();
    if bindings.is_empty() {
        return None;
    }
    Some(CoreActionData::SyncReactionRoles {
        guild_id: guild_id.to_owned(),
        channel_id: panel.channel_id.clone(),
        message_id: panel.message_id.clone(),
        bindings,
        user_id: user_id.map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::reaction_index;

    #[test]
    fn matches_number_reactions_with_or_without_emoji_presentation() {
        assert_eq!(reaction_index("3\u{fe0f}\u{20e3}"), Some(2));
        assert_eq!(reaction_index("3\u{20e3}"), Some(2));
        assert_eq!(reaction_index("3"), None);
    }
}
