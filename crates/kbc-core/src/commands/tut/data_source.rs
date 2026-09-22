//! 敵名・別称と関連Assetを共有HTTP Service経由で取得する。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::commands::common::remote_data::{
    CachedTextResource, RemoteAssetSource, RemoteDataError,
};
use crate::services::HttpService;

use super::{EnemySearchData, EnemySearchEntry};

const SITE_DATA_BASE: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata";
const ENEMY_NAMES_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/res/Enemyname.tsv";
const ENEMY_ALIASES_URL: &str =
    "https://raw.githubusercontent.com/Sugar2550/omoroirie/main/data/enemyname.json";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize)]
struct EnemyAliasEntry {
    id: usize,
    names: Vec<String>,
}

#[derive(Clone)]
pub(super) struct TutDataSource {
    names: Arc<CachedTextResource<Vec<Option<String>>>>,
    aliases: Arc<CachedTextResource<Vec<EnemyAliasEntry>>>,
    assets: RemoteAssetSource,
}

impl TutDataSource {
    pub(super) fn new(http: Arc<HttpService>) -> Self {
        Self {
            names: Arc::new(CachedTextResource::new(
                "Enemyname.tsv",
                ENEMY_NAMES_URL,
                CACHE_TTL,
                Arc::clone(&http),
                parse_enemy_names,
            )),
            aliases: Arc::new(CachedTextResource::new(
                "enemyname.json",
                ENEMY_ALIASES_URL,
                CACHE_TTL,
                Arc::clone(&http),
                parse_enemy_aliases,
            )),
            assets: RemoteAssetSource::new(SITE_DATA_BASE, CACHE_TTL, http),
        }
    }

    pub(super) async fn fetch_search_data(&self) -> Result<EnemySearchData, RemoteDataError> {
        let (names, aliases) = tokio::try_join!(self.names.fetch(), self.aliases.fetch())?;
        let aliases_by_id = aliases
            .iter()
            .map(|entry| (entry.id, entry.names.as_slice()))
            .collect::<HashMap<_, _>>();
        let mut entries = Vec::new();
        let mut id_index = HashMap::new();
        for (id, display_name) in names.iter().enumerate() {
            let Some(display_name) = display_name else {
                continue;
            };
            let mut entry_aliases = Vec::new();
            for alias in aliases_by_id.get(&id).copied().unwrap_or_default() {
                if alias != display_name && !entry_aliases.contains(alias) {
                    entry_aliases.push(alias.clone());
                }
            }
            let index = entries.len();
            entries.push(EnemySearchEntry {
                id,
                display_name: display_name.clone(),
                aliases: entry_aliases,
            });
            id_index.insert(id, index);
        }
        Ok(EnemySearchData { entries, id_index })
    }

    pub(super) fn assets(&self) -> RemoteAssetSource {
        self.assets.clone()
    }
}

fn parse_enemy_names(text: &str) -> Result<Vec<Option<String>>, String> {
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return Err("rows are required".to_owned());
    }
    let mut names = Vec::with_capacity(lines.len());
    for (index, line) in lines.into_iter().enumerate() {
        if line.contains('\t') || line.chars().any(|character| character.is_control()) {
            return Err(format!("row {} is invalid", index + 1));
        }
        names.push((!line.trim().is_empty()).then(|| line.to_owned()));
    }
    if !names.iter().any(Option::is_some) {
        return Err("searchable names are required".to_owned());
    }
    Ok(names)
}

fn parse_enemy_aliases(text: &str) -> Result<Vec<EnemyAliasEntry>, String> {
    let entries: Vec<EnemyAliasEntry> =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    if entries.is_empty() {
        return Err("a non-empty array is required".to_owned());
    }
    let mut seen = std::collections::HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if !seen.insert(entry.id) || entry.names.iter().any(|name| name.trim().is_empty()) {
            return Err(format!("entry {index} is invalid"));
        }
    }
    Ok(entries)
}
