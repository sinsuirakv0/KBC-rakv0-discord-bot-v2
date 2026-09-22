//! sale Commandが利用するRemote dataを共通HttpService経由で取得する。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::services::{HttpService, HttpServiceError};

use super::super::common::schedule::validate_header;
use super::model::{OrderedNames, SaleDisplayData, SaleJson};

const SALE_JSON_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/sale.json";
const SALE_NAMES_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/sale_name.csv";
const ALL_DAY_EVENT_NAMES_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/res/All_day_event.tsv";
const MISSION_NAMES_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/res/Mission_Name.csv";
const CARD_SETTING_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/setting/cardsetting";

pub(crate) struct SaleDataSource {
    http: Arc<HttpService>,
}

impl SaleDataSource {
    pub(crate) fn new(http: Arc<HttpService>) -> Self {
        Self { http }
    }

    pub(super) async fn fetch_sale_json(&self) -> Result<SaleJson, SaleDataError> {
        parse_sale_json(&self.http.get_text(SALE_JSON_URL).await?)
    }

    pub(super) async fn fetch_display_data(&self) -> Result<SaleDisplayData, SaleDataError> {
        let sale = self.fetch_sale_json().await?;
        self.fetch_display_data_for(sale).await
    }

    pub(in crate::commands) async fn fetch_display_data_for(
        &self,
        sale: SaleJson,
    ) -> Result<SaleDisplayData, SaleDataError> {
        let (sale_names, all_day_names, mission_names, card_setting) = tokio::try_join!(
            self.http.get_text(SALE_NAMES_URL),
            self.http.get_text(ALL_DAY_EVENT_NAMES_URL),
            self.http.get_text(MISSION_NAMES_URL),
            self.http.get_text(CARD_SETTING_URL),
        )?;
        Ok(SaleDisplayData {
            sale,
            sale_names: parse_id_names(&sale_names),
            all_day_event_names: parse_all_day_names(&all_day_names),
            mission_names: parse_id_names(&mission_names),
            card_setting_stage_ids: parse_card_setting(&card_setting),
        })
    }
}

fn parse_sale_json(text: &str) -> Result<SaleJson, SaleDataError> {
    let sale: SaleJson = serde_json::from_str(text)?;
    for entry in &sale.data {
        validate_header(&entry.header).map_err(SaleDataError::Invalid)?;
    }
    Ok(sale)
}

fn parse_id_names(text: &str) -> OrderedNames {
    let mut names = OrderedNames::new();
    for raw_line in text.lines() {
        let line = raw_line.trim_start_matches('\u{feff}');
        let Some((id, name)) = line.split_once(',') else {
            continue;
        };
        if let (Ok(id), false) = (id.trim().parse(), name.trim().is_empty()) {
            names.insert(id, name.trim().to_owned());
        }
    }
    names
}

fn parse_all_day_names(text: &str) -> OrderedNames {
    let mut names = OrderedNames::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let mut cells = line.split('\t');
        let Some(name) = cells.next().map(str::trim).filter(|name| !name.is_empty()) else {
            continue;
        };
        let Some(id) = cells.next().and_then(|value| value.trim().parse().ok()) else {
            continue;
        };
        names.insert(id, name.to_owned());
    }
    names
}

fn parse_card_setting(text: &str) -> Vec<i64> {
    let mut ids = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line
            .split('#')
            .next()
            .unwrap_or("")
            .split("//")
            .next()
            .unwrap_or("")
            .trim();
        for token in line.split(|character: char| character == ',' || character.is_whitespace()) {
            if let Ok(id) = token.trim().parse()
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    ids
}

#[derive(Debug)]
pub(in crate::commands) enum SaleDataError {
    Http(HttpServiceError),
    Json(serde_json::Error),
    Invalid(String),
}

impl Display for SaleDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(error) => Display::fmt(error, formatter),
            Self::Json(error) => write!(formatter, "sale JSON is invalid: {error}"),
            Self::Invalid(reason) => write!(formatter, "sale data is invalid: {reason}"),
        }
    }
}

impl Error for SaleDataError {}

impl From<HttpServiceError> for SaleDataError {
    fn from(error: HttpServiceError) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for SaleDataError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
