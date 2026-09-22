//! item Commandが利用するRemote dataを共通HttpService経由で取得する。

use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::services::{HttpService, HttpServiceError};

use super::super::common::schedule::validate_header;
use super::model::{ItemDisplayData, ItemJson, ItemName};

const ITEM_JSON_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/item.json";
const ITEM_NAMES_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/item_name.csv";
const SALE_NAMES_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/sale_name.csv";

pub(crate) struct ItemDataSource {
    http: Arc<HttpService>,
}

impl ItemDataSource {
    pub(crate) fn new(http: Arc<HttpService>) -> Self {
        Self { http }
    }

    pub(super) async fn fetch_item_json(&self) -> Result<ItemJson, ItemDataError> {
        parse_item_json(&self.http.get_text(ITEM_JSON_URL).await?)
    }

    pub(super) async fn fetch_display_data(&self) -> Result<ItemDisplayData, ItemDataError> {
        let item = self.fetch_item_json().await?;
        self.fetch_display_data_for(item).await
    }

    pub(in crate::commands) async fn fetch_display_data_for(
        &self,
        item: ItemJson,
    ) -> Result<ItemDisplayData, ItemDataError> {
        let (item_names_text, sale_names_text) = tokio::try_join!(
            self.http.get_text(ITEM_NAMES_URL),
            self.http.get_text(SALE_NAMES_URL),
        )?;
        Ok(ItemDisplayData {
            item,
            item_names: parse_item_names(&item_names_text)?,
            sale_names: parse_id_names(&sale_names_text)?,
        })
    }
}

fn parse_item_json(text: &str) -> Result<ItemJson, ItemDataError> {
    let item: ItemJson = serde_json::from_str(text)?;
    for entry in &item.data {
        validate_header(&entry.header).map_err(ItemDataError::Invalid)?;
    }
    Ok(item)
}

fn parse_item_names(text: &str) -> Result<HashMap<i64, ItemName>, ItemDataError> {
    let mut names = HashMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim_start_matches('\u{feff}');
        if line.trim().is_empty() {
            continue;
        }
        let mut cells = line.split(',');
        let Some(id) = cells.next().and_then(|value| value.trim().parse().ok()) else {
            continue;
        };
        let Some(name) = cells.next().map(str::trim).filter(|name| !name.is_empty()) else {
            continue;
        };
        names.insert(
            id,
            ItemName {
                name: name.to_owned(),
                detail: cells.collect::<Vec<_>>().join(",").trim().to_owned(),
            },
        );
    }
    if names.is_empty() {
        return Err(ItemDataError::Invalid(
            "item name CSV contains no valid entries".to_owned(),
        ));
    }
    Ok(names)
}

fn parse_id_names(text: &str) -> Result<HashMap<i64, String>, ItemDataError> {
    let mut names = HashMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim_start_matches('\u{feff}');
        let Some((id, name)) = line.split_once(',') else {
            continue;
        };
        if let (Ok(id), false) = (id.trim().parse(), name.trim().is_empty()) {
            names.insert(id, name.trim().to_owned());
        }
    }
    if names.is_empty() {
        return Err(ItemDataError::Invalid(
            "sale name CSV contains no valid entries".to_owned(),
        ));
    }
    Ok(names)
}

#[derive(Debug)]
pub(in crate::commands) enum ItemDataError {
    Http(HttpServiceError),
    Json(serde_json::Error),
    Invalid(String),
}

impl Display for ItemDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(error) => Display::fmt(error, formatter),
            Self::Json(error) => write!(formatter, "item JSON is invalid: {error}"),
            Self::Invalid(reason) => write!(formatter, "item data is invalid: {reason}"),
        }
    }
}

impl Error for ItemDataError {}

impl From<HttpServiceError> for ItemDataError {
    fn from(error: HttpServiceError) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for ItemDataError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
