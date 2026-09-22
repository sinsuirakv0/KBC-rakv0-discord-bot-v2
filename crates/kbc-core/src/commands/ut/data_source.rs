//! 味方キャラ索引・UnitBuyと関連Assetを共有HTTP Service経由で取得する。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::commands::common::remote_data::{
    CachedTextResource, RemoteAssetSource, RemoteDataError,
};
use crate::services::HttpService;

use super::{CharacterForm, CharacterIndex, CharacterUnit, UnitBuy, UnitBuyEntry};

const SITE_DATA_BASE: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata";
const CHARACTER_INDEX_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/character-index.json";
const UNIT_BUY_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/Data/unitbuy.csv";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize)]
struct RawCharacterIndex {
    units: Vec<RawCharacterUnit>,
}

#[derive(Deserialize)]
struct RawCharacterUnit {
    id: String,
    forms: Vec<RawCharacterForm>,
    aliases: Vec<String>,
}

#[derive(Deserialize)]
struct RawCharacterForm {
    name: String,
    description: String,
}

#[derive(Clone)]
pub(super) struct UtDataSource {
    character_index: Arc<CachedTextResource<CharacterIndex>>,
    unit_buy: Arc<CachedTextResource<UnitBuy>>,
    assets: RemoteAssetSource,
}

impl UtDataSource {
    pub(super) fn new(http: Arc<HttpService>) -> Self {
        Self {
            character_index: Arc::new(CachedTextResource::new(
                "character-index.json",
                CHARACTER_INDEX_URL,
                CACHE_TTL,
                Arc::clone(&http),
                parse_character_index,
            )),
            unit_buy: Arc::new(CachedTextResource::new(
                "unitbuy.csv",
                UNIT_BUY_URL,
                CACHE_TTL,
                Arc::clone(&http),
                parse_unit_buy,
            )),
            assets: RemoteAssetSource::new(SITE_DATA_BASE, CACHE_TTL, http),
        }
    }

    pub(super) async fn fetch_character_index(
        &self,
    ) -> Result<Arc<CharacterIndex>, RemoteDataError> {
        self.character_index.fetch().await
    }

    pub(super) async fn fetch_unit_buy(&self) -> Result<Arc<UnitBuy>, RemoteDataError> {
        self.unit_buy.fetch().await
    }

    pub(super) fn assets(&self) -> RemoteAssetSource {
        self.assets.clone()
    }
}

fn parse_character_index(text: &str) -> Result<CharacterIndex, String> {
    let raw: RawCharacterIndex = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if raw.units.is_empty() {
        return Err("units must be a non-empty array".to_owned());
    }
    let mut units = Vec::with_capacity(raw.units.len());
    for (index, unit) in raw.units.into_iter().enumerate() {
        let expected_id = format!("{index:03}");
        if unit.id != expected_id {
            return Err(format!("unit {index} has an invalid id"));
        }
        if !(1..=4).contains(&unit.forms.len()) {
            return Err(format!("unit {} forms must contain 1-4 items", unit.id));
        }
        if unit.forms.iter().any(|form| form.name.trim().is_empty()) {
            return Err(format!("unit {} contains an invalid form", unit.id));
        }
        if unit.aliases.iter().any(|alias| alias.trim().is_empty())
            || unit.aliases.iter().collect::<HashSet<_>>().len() != unit.aliases.len()
        {
            return Err(format!("unit {} aliases are invalid", unit.id));
        }
        units.push(CharacterUnit {
            id: unit.id,
            forms: unit
                .forms
                .into_iter()
                .map(|form| {
                    let _ = form.description;
                    CharacterForm { name: form.name }
                })
                .collect(),
            aliases: unit.aliases,
        });
    }
    Ok(CharacterIndex { units })
}

fn parse_unit_buy(text: &str) -> Result<UnitBuy, String> {
    let mut units = Vec::new();
    for (index, line) in text
        .trim_start_matches('\u{feff}')
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let columns = line.split(',').collect::<Vec<_>>();
        if columns.len() < 63 {
            return Err(format!("row {index} has too few columns"));
        }
        let mut shared_form_ids: [Option<String>; 2] = [None, None];
        for (form_index, column) in columns[61..=62].iter().enumerate() {
            let value = column
                .parse::<i64>()
                .map_err(|_| format!("row {index} form {form_index} is invalid"))?;
            if value < -1 {
                return Err(format!("row {index} form {form_index} is invalid"));
            }
            shared_form_ids[form_index] = (value != -1).then(|| format!("{value:03}"));
        }
        units.push(UnitBuyEntry {
            id: format!("{index:03}"),
            shared_form_ids,
        });
    }
    if units.is_empty() {
        return Err("no rows".to_owned());
    }
    Ok(UnitBuy { units })
}
