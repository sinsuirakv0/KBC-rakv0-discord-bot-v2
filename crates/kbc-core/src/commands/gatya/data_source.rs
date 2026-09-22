//! gatya Commandが利用するRemote dataを共通HttpService経由で取得する。

use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::services::{HttpService, HttpServiceError};

use super::super::common::schedule::validate_header;
use super::model::{
    GachaJson, GachaJsonWithMappings, GachaLookupData, GachaScheduleData, ItemScheduleJson,
    MappingMaps, ModeMaps, NameMaps,
};

const GACHA_JSON_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya.json";
const ITEM_JSON_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/item.json";
const SALE_NAMES_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/sale_name.csv";
const GACHA_NAME_URLS: ModeUrls = ModeUrls {
    rare: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_name.csv",
    event: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_e_name.csv",
    normal: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_n_name.csv",
};
const SERIES_NAME_URLS: ModeUrls = ModeUrls {
    rare: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_series_name.csv",
    event: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_e_series_name.csv",
    normal: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_n_series_name.csv",
};
const SHORT_SERIES_NAME_URLS: ModeUrls = ModeUrls {
    rare: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_series_name_omit.csv",
    event: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_e_series_name_omit.csv",
    normal: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/gatya_n_series_nameomit.csv",
};
const SERIES_MAPPING_URLS: ModeUrls = ModeUrls {
    rare: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/Data/GatyaData_Option_SetR.tsv",
    event: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/Data/GatyaData_Option_SetE.tsv",
    normal: "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/Data/GatyaData_Option_SetN.tsv",
};

struct ModeUrls {
    rare: &'static str,
    event: &'static str,
    normal: &'static str,
}

pub(crate) struct GatyaDataSource {
    http: Arc<HttpService>,
}

impl GatyaDataSource {
    pub(crate) fn new(http: Arc<HttpService>) -> Self {
        Self { http }
    }

    pub(super) async fn fetch_gacha_json(&self) -> Result<GachaJson, GatyaDataError> {
        parse_gacha_json(&self.http.get_text(GACHA_JSON_URL).await?)
    }

    pub(super) async fn fetch_json_with_mappings(
        &self,
    ) -> Result<GachaJsonWithMappings, GatyaDataError> {
        let (gacha, series_mappings) =
            tokio::try_join!(self.fetch_gacha_json(), self.fetch_mappings())?;
        Ok(GachaJsonWithMappings {
            gacha,
            series_mappings,
        })
    }

    pub(super) async fn fetch_schedule_data(&self) -> Result<GachaScheduleData, GatyaDataError> {
        let gacha = self.fetch_gacha_json().await?;
        self.fetch_schedule_data_for(gacha).await
    }

    pub(in crate::commands) async fn fetch_schedule_data_for(
        &self,
        gacha: GachaJson,
    ) -> Result<GachaScheduleData, GatyaDataError> {
        let (item, sale_names_text, gacha_names, optional_short_names, series_mappings) = tokio::try_join!(
            self.fetch_item_json(),
            async {
                self.http
                    .get_text(SALE_NAMES_URL)
                    .await
                    .map_err(GatyaDataError::from)
            },
            self.fetch_names(&GACHA_NAME_URLS),
            self.fetch_optional_names(&SHORT_SERIES_NAME_URLS),
            self.fetch_mappings(),
        )?;
        let derived_names = derive_series_names(&gacha_names, &series_mappings);
        Ok(GachaScheduleData {
            gacha,
            item,
            sale_names: parse_id_names(&sale_names_text),
            short_series_names: overlay_names(derived_names, optional_short_names),
            series_mappings,
        })
    }

    pub(super) async fn fetch_lookup_data(&self) -> Result<GachaLookupData, GatyaDataError> {
        let (gacha, gacha_names, series_names, optional_short_names, series_mappings) = tokio::try_join!(
            self.fetch_gacha_json(),
            self.fetch_names(&GACHA_NAME_URLS),
            self.fetch_optional_names(&SERIES_NAME_URLS),
            self.fetch_optional_names(&SHORT_SERIES_NAME_URLS),
            self.fetch_mappings(),
        )?;
        let derived_names = derive_series_names(&gacha_names, &series_mappings);
        Ok(GachaLookupData {
            gacha,
            gacha_names,
            series_names,
            short_series_names: overlay_names(derived_names, optional_short_names),
            series_mappings,
        })
    }

    async fn fetch_item_json(&self) -> Result<ItemScheduleJson, GatyaDataError> {
        let item: ItemScheduleJson =
            serde_json::from_str(&self.http.get_text(ITEM_JSON_URL).await?)?;
        for entry in &item.data {
            validate_header(&entry.header).map_err(GatyaDataError::Invalid)?;
        }
        Ok(item)
    }

    async fn fetch_names(&self, urls: &ModeUrls) -> Result<NameMaps, GatyaDataError> {
        let (rare, event, normal) = tokio::try_join!(
            self.http.get_text(urls.rare),
            self.http.get_text(urls.event),
            self.http.get_text(urls.normal),
        )?;
        Ok(ModeMaps {
            rare: parse_id_names(&rare),
            event: parse_id_names(&event),
            normal: parse_id_names(&normal),
        })
    }

    async fn fetch_optional_names(&self, urls: &ModeUrls) -> Result<NameMaps, GatyaDataError> {
        let (rare, event, normal) = tokio::try_join!(
            self.http.get_optional_text(urls.rare),
            self.http.get_optional_text(urls.event),
            self.http.get_optional_text(urls.normal),
        )?;
        Ok(ModeMaps {
            rare: rare.map(|text| parse_id_names(&text)).unwrap_or_default(),
            event: event.map(|text| parse_id_names(&text)).unwrap_or_default(),
            normal: normal.map(|text| parse_id_names(&text)).unwrap_or_default(),
        })
    }

    async fn fetch_mappings(&self) -> Result<MappingMaps, GatyaDataError> {
        let (rare, event, normal) = tokio::try_join!(
            self.http.get_text(SERIES_MAPPING_URLS.rare),
            self.http.get_text(SERIES_MAPPING_URLS.event),
            self.http.get_text(SERIES_MAPPING_URLS.normal),
        )?;
        Ok(ModeMaps {
            rare: parse_series_mapping(&rare)?,
            event: parse_series_mapping(&event)?,
            normal: parse_series_mapping(&normal)?,
        })
    }
}

fn parse_gacha_json(text: &str) -> Result<GachaJson, GatyaDataError> {
    let gacha: GachaJson = serde_json::from_str(text)?;
    for block in &gacha.data {
        validate_header(&block.header.schedule).map_err(GatyaDataError::Invalid)?;
    }
    Ok(gacha)
}

fn parse_id_names(text: &str) -> HashMap<i64, String> {
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
    names
}

fn parse_series_mapping(text: &str) -> Result<HashMap<i64, i64>, GatyaDataError> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let headers = lines
        .next()
        .map(|line| line.split('\t').map(str::trim).collect::<Vec<_>>())
        .unwrap_or_default();
    let gacha_index = headers
        .iter()
        .position(|header| *header == "GatyaSetID")
        .ok_or_else(|| GatyaDataError::Invalid("GatyaSetID column is missing".to_owned()))?;
    let series_index = headers
        .iter()
        .position(|header| *header == "seriesID")
        .ok_or_else(|| GatyaDataError::Invalid("seriesID column is missing".to_owned()))?;
    let mut mappings = HashMap::new();
    for line in lines {
        let cells = line.split('\t').collect::<Vec<_>>();
        let values = cells
            .get(gacha_index)
            .zip(cells.get(series_index))
            .and_then(|(gacha_id, series_id)| {
                Some((
                    gacha_id.trim().parse().ok()?,
                    series_id.trim().parse().ok()?,
                ))
            });
        if let Some((gacha_id, series_id)) = values {
            mappings.insert(gacha_id, series_id);
        }
    }
    Ok(mappings)
}

fn derive_series_names(gacha_names: &NameMaps, mappings: &MappingMaps) -> NameMaps {
    ModeMaps {
        rare: derive_mode_names(&gacha_names.rare, &mappings.rare),
        event: derive_mode_names(&gacha_names.event, &mappings.event),
        normal: derive_mode_names(&gacha_names.normal, &mappings.normal),
    }
}

fn derive_mode_names(
    gacha_names: &HashMap<i64, String>,
    mappings: &HashMap<i64, i64>,
) -> HashMap<i64, String> {
    let mut rows = mappings.iter().collect::<Vec<_>>();
    rows.sort_by_key(|(gacha_id, _)| **gacha_id);
    let mut names = HashMap::new();
    for (gacha_id, series_id) in rows {
        if names.contains_key(series_id) {
            continue;
        }
        if let Some(name) = gacha_names.get(gacha_id) {
            names.insert(*series_id, name.clone());
        }
    }
    names
}

fn overlay_names(mut fallback: NameMaps, preferred: NameMaps) -> NameMaps {
    fallback.rare.extend(preferred.rare);
    fallback.event.extend(preferred.event);
    fallback.normal.extend(preferred.normal);
    fallback
}

#[derive(Debug)]
pub(in crate::commands) enum GatyaDataError {
    Http(HttpServiceError),
    Json(serde_json::Error),
    Invalid(String),
}

impl Display for GatyaDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(error) => Display::fmt(error, formatter),
            Self::Json(error) => write!(formatter, "gatya JSON is invalid: {error}"),
            Self::Invalid(reason) => write!(formatter, "gatya data is invalid: {reason}"),
        }
    }
}

impl Error for GatyaDataError {}

impl From<HttpServiceError> for GatyaDataError {
    fn from(error: HttpServiceError) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for GatyaDataError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
