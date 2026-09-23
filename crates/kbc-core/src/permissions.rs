//! 固定Bot管理者と、Storageで管理する全サーバー共通メンテナー権限を判定する。

use crate::command::CommandContext;
use crate::storage::{StorageError, StorageService};

const ADMINISTRATOR_IDS: [&str; 3] = [
    "1447045405257760820",
    "1347420765410295928",
    "1138400546823340102",
];

pub(crate) fn is_bot_administrator(user_id: &str) -> bool {
    ADMINISTRATOR_IDS.contains(&user_id)
}

pub(crate) async fn has_maintainer_access(
    storage: &StorageService,
    context: &CommandContext,
) -> Result<bool, StorageError> {
    if is_bot_administrator(context.user_id()) {
        return Ok(true);
    }
    storage
        .is_maintainer(context.user_id(), context.member_role_ids())
        .await
}
