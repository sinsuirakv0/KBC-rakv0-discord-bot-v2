//! item.jsonと表示名Dataの検証済みModel。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::super::common::schedule::{ScheduleHeader, TimeBlock};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct ItemGift {
    pub(in crate::commands) event_id: i64,
    pub(in crate::commands) gift_type: i64,
    pub(in crate::commands) gift_amount: i64,
    pub(in crate::commands) title: String,
    pub(in crate::commands) message: String,
    pub(in crate::commands) url: String,
    pub(in crate::commands) repeat_flag: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct ItemEntry {
    pub(in crate::commands) header: ScheduleHeader,
    pub(in crate::commands) time_blocks: Vec<TimeBlock>,
    pub(in crate::commands) gift: ItemGift,
    #[serde(default, skip_serializing)]
    pub(in crate::commands) raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct ItemJson {
    #[serde(default, rename = "updatedAt")]
    pub(in crate::commands) _updated_at: String,
    pub(in crate::commands) data: Vec<ItemEntry>,
}

pub(in crate::commands) struct ItemName {
    pub(in crate::commands) name: String,
    pub(in crate::commands) detail: String,
}

pub(in crate::commands) struct ItemDisplayData {
    pub(in crate::commands) item: ItemJson,
    pub(in crate::commands) item_names: HashMap<i64, ItemName>,
    pub(in crate::commands) sale_names: HashMap<i64, String>,
}
