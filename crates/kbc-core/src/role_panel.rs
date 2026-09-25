//! 永続的な通知ロール選択パネルのReactionをRole操作へ変換する。

use std::sync::Arc;

use kbc_protocol::CoreActionData;

use crate::storage::StorageService;

const NUMBER_EMOJIS: [&str; 9] = ["1️⃣", "2️⃣", "3️⃣", "4️⃣", "5️⃣", "6️⃣", "7️⃣", "8️⃣", "9️⃣"];

pub(crate) struct RolePanelService {
    storage: Arc<StorageService>,
}

impl RolePanelService {
    pub(crate) fn new(storage: Arc<StorageService>) -> Self {
        Self { storage }
    }

    pub(crate) async fn action_for_reaction(
        &self,
        guild_id: Option<&str>,
        channel_id: &str,
        message_id: &str,
        user_id: &str,
        emoji: &str,
        added: bool,
    ) -> Option<CoreActionData> {
        let guild_id = guild_id?;
        let index = NUMBER_EMOJIS.iter().position(|current| *current == emoji)?;
        let role_id = match self
            .storage
            .notification_role_for_reaction(guild_id, channel_id, message_id, index)
            .await
        {
            Ok(role_id) => role_id?,
            Err(error) => {
                eprintln!("Notification role lookup failed: {error}");
                return None;
            }
        };
        Some(if added {
            CoreActionData::AddGuildMemberRole {
                guild_id: guild_id.to_owned(),
                user_id: user_id.to_owned(),
                role_id,
            }
        } else {
            CoreActionData::RemoveGuildMemberRole {
                guild_id: guild_id.to_owned(),
                user_id: user_id.to_owned(),
                role_id,
            }
        })
    }
}

pub(crate) fn number_emoji(index: usize) -> Option<&'static str> {
    NUMBER_EMOJIS.get(index).copied()
}
