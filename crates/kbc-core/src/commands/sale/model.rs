//! sale.jsonと表示名Dataの検証済みModel。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::super::common::schedule::{ScheduleHeader, TimeBlock};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct SaleEntry {
    pub(in crate::commands) header: ScheduleHeader,
    pub(in crate::commands) time_blocks: Vec<TimeBlock>,
    pub(in crate::commands) stage_ids: Vec<i64>,
    #[serde(default, skip_serializing)]
    pub(in crate::commands) raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands) struct SaleJson {
    #[serde(default, rename = "updatedAt")]
    pub(in crate::commands) _updated_at: String,
    pub(in crate::commands) data: Vec<SaleEntry>,
}

pub(in crate::commands) struct OrderedNames {
    values: HashMap<i64, String>,
    order: Vec<i64>,
}

impl OrderedNames {
    pub(in crate::commands) fn new() -> Self {
        Self {
            values: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub(in crate::commands) fn insert(&mut self, id: i64, name: String) {
        if !self.values.contains_key(&id) {
            self.order.push(id);
        }
        self.values.insert(id, name);
    }

    pub(in crate::commands) fn get(&self, id: i64) -> Option<&str> {
        self.values.get(&id).map(String::as_str)
    }

    pub(super) fn entries(&self) -> impl Iterator<Item = (i64, &str)> {
        self.order
            .iter()
            .filter_map(|id| self.get(*id).map(|name| (*id, name)))
    }
}

pub(in crate::commands) struct SaleDisplayData {
    pub(in crate::commands) sale: SaleJson,
    pub(in crate::commands) sale_names: OrderedNames,
    pub(in crate::commands) all_day_event_names: OrderedNames,
    pub(in crate::commands) mission_names: OrderedNames,
    pub(in crate::commands) card_setting_stage_ids: Vec<i64>,
}

impl SaleDisplayData {
    pub(super) fn merged_search_names(&self) -> Vec<(i64, &str)> {
        let mut seen = HashSet::new();
        self.sale_names
            .entries()
            .chain(self.all_day_event_names.entries())
            .filter(|(id, _)| seen.insert(*id))
            .collect()
    }
}
