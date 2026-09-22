//! gatya.jsonとSeries関連Dataの検証済みModel。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::super::common::schedule::ScheduleHeader;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::commands) enum GachaMode {
    Rare,
    Event,
    Normal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct GachaRate {
    pub(in crate::commands) normal: Number,
    pub(in crate::commands) rare: Number,
    pub(in crate::commands) super_rare: Number,
    pub(in crate::commands) uber_rare: Number,
    pub(in crate::commands) legend_rare: Number,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct GachaEntry {
    pub(in crate::commands) id: i64,
    pub(in crate::commands) price: Number,
    pub(in crate::commands) flags: i64,
    pub(in crate::commands) rates: GachaRate,
    pub(in crate::commands) guaranteed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::commands) message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct GachaHeader {
    #[serde(flatten)]
    pub(in crate::commands) schedule: ScheduleHeader,
    pub(in crate::commands) gacha_type: i64,
    pub(in crate::commands) gacha_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct GachaBlock {
    pub(in crate::commands) header: GachaHeader,
    pub(in crate::commands) gachas: Vec<GachaEntry>,
    #[serde(default, skip_serializing)]
    pub(in crate::commands) raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct GachaJson {
    #[serde(default, rename = "updatedAt")]
    pub(in crate::commands) _updated_at: String,
    pub(in crate::commands) data: Vec<GachaBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct ItemScheduleGift {
    pub(in crate::commands) gift_type: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct ItemScheduleEntry {
    pub(in crate::commands) header: ScheduleHeader,
    pub(in crate::commands) gift: ItemScheduleGift,
}

#[derive(Debug, Deserialize)]
pub(in crate::commands) struct ItemScheduleJson {
    pub(in crate::commands) data: Vec<ItemScheduleEntry>,
}

pub(in crate::commands) struct ModeMaps<T> {
    pub(in crate::commands) rare: T,
    pub(in crate::commands) event: T,
    pub(in crate::commands) normal: T,
}

impl<T> ModeMaps<T> {
    pub(in crate::commands) fn get(&self, mode: GachaMode) -> &T {
        match mode {
            GachaMode::Rare => &self.rare,
            GachaMode::Event => &self.event,
            GachaMode::Normal => &self.normal,
        }
    }
}

pub(in crate::commands) type NameMaps = ModeMaps<HashMap<i64, String>>;
pub(in crate::commands) type MappingMaps = ModeMaps<HashMap<i64, i64>>;

pub(in crate::commands) struct GachaScheduleData {
    pub(in crate::commands) gacha: GachaJson,
    pub(in crate::commands) item: ItemScheduleJson,
    pub(in crate::commands) sale_names: HashMap<i64, String>,
    pub(in crate::commands) short_series_names: NameMaps,
    pub(in crate::commands) series_mappings: MappingMaps,
}

pub(super) struct GachaLookupData {
    pub(super) gacha: GachaJson,
    pub(super) gacha_names: NameMaps,
    pub(super) series_names: NameMaps,
    pub(super) short_series_names: NameMaps,
    pub(super) series_mappings: MappingMaps,
}

pub(super) struct GachaJsonWithMappings {
    pub(super) gacha: GachaJson,
    pub(super) series_mappings: MappingMaps,
}
