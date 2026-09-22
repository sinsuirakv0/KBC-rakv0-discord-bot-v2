//! ステージ検索用のRemote dataを検証し、有限なSnapshotとして保持する。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::services::{ConditionalTextResponse, HttpService, HttpServiceError};

const ASSET_RESOURCE_BASE: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/res";
const CHAPTER_STAGE_BASE: &str = "https://raw.githubusercontent.com/Sugar2550/omoroirie/main/data";
const STAGE_TYPES_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/stage_type.csv";
const MAP_NAMES_URL: &str = "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-assets/main/jp/sitedata/res/Map_Name.csv";
const SALE_NAMES_URL: &str =
    "https://raw.githubusercontent.com/sinsuirakv0/KBC-rakv0-event/main/data/sale_name.csv";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_STAGE_TYPES: usize = 64;
const MAX_NAME_ROWS: usize = 100_000;
const MAX_STAGES_PER_ROW: usize = 1_000;
const MAX_NAME_UTF16: usize = 200;

#[derive(Clone)]
pub(super) enum StageKind {
    Map { search_names: Vec<String> },
    Stage { stage_index: usize },
}

#[derive(Clone)]
pub(super) struct StageEntry {
    pub(super) kind: StageKind,
    pub(super) raw_map_id: usize,
    pub(super) display_id: String,
    pub(super) display_name: String,
    pub(super) jdb_type: String,
    pub(super) jdb_map: usize,
    display_type: Option<String>,
    display_map: Option<usize>,
}

impl StageEntry {
    pub(super) fn stage_index(&self) -> Option<usize> {
        match self.kind {
            StageKind::Map { .. } => None,
            StageKind::Stage { stage_index } => Some(stage_index),
        }
    }

    pub(super) fn search_names(&self) -> &[String] {
        match &self.kind {
            StageKind::Map { search_names } => search_names,
            StageKind::Stage { .. } => std::slice::from_ref(&self.display_name),
        }
    }

    pub(super) fn is_map(&self) -> bool {
        matches!(self.kind, StageKind::Map { .. })
    }
}

pub(super) struct StageSearchData {
    pub(super) maps: Vec<Arc<StageEntry>>,
    pub(super) stages: Vec<Arc<StageEntry>>,
    display_types: Vec<String>,
    id_index: HashMap<String, Arc<StageEntry>>,
}

impl StageSearchData {
    pub(super) fn find_by_id(&self, query: &str) -> Option<Arc<StageEntry>> {
        let key = parse_id_key(query.trim(), &self.display_types)?;
        self.id_index.get(&key).cloned()
    }
}

#[derive(Clone)]
struct ResourceState {
    text: Arc<str>,
    etag: Option<String>,
    last_modified: Option<String>,
}

struct StageSnapshot {
    value: Arc<StageSearchData>,
    resources: HashMap<String, ResourceState>,
    validated_at: Instant,
}

#[derive(Clone)]
pub(super) struct StageDataSource {
    http: Arc<HttpService>,
    snapshot: Arc<Mutex<Option<StageSnapshot>>>,
}

impl StageDataSource {
    pub(super) fn new(http: Arc<HttpService>) -> Self {
        Self {
            http,
            snapshot: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) async fn fetch(&self) -> Result<Arc<StageSearchData>, StageDataError> {
        let mut snapshot = self.snapshot.lock().await;
        let now = Instant::now();
        if let Some(current) = snapshot.as_ref()
            && now.duration_since(current.validated_at) < CACHE_TTL
        {
            return Ok(Arc::clone(&current.value));
        }

        match refresh(&self.http, snapshot.as_ref(), now).await {
            Ok(next) => {
                let value = Arc::clone(&next.value);
                *snapshot = Some(next);
                Ok(value)
            }
            Err(error) => match snapshot.as_ref() {
                Some(stale) => {
                    eprintln!("Stage data refresh failed; stale snapshot is used: {error}");
                    Ok(Arc::clone(&stale.value))
                }
                None => Err(error),
            },
        }
    }
}

async fn refresh(
    http: &Arc<HttpService>,
    previous: Option<&StageSnapshot>,
    now: Instant,
) -> Result<StageSnapshot, StageDataError> {
    let previous_resources = previous
        .map(|snapshot| snapshot.resources.clone())
        .unwrap_or_default();
    let stage_types = fetch_resource(
        http,
        STAGE_TYPES_URL,
        previous_resources.get(STAGE_TYPES_URL).cloned(),
    )
    .await?;
    let ranges = parse_stage_types(&stage_types.text)?;
    let mut resources = HashMap::from([(STAGE_TYPES_URL.to_owned(), stage_types)]);

    let mut requests = vec![
        DocumentRequest::single("map", MAP_NAMES_URL),
        DocumentRequest::single("sale", SALE_NAMES_URL),
    ];
    for range in std::iter::once(StageTypeRange {
        from: 0,
        to: 999,
        type_name: "N".to_owned(),
    })
    .chain(ranges.iter().cloned())
    {
        requests.push(DocumentRequest {
            key: format!("normal:{}", range.type_name),
            candidates: vec![
                join_url(
                    ASSET_RESOURCE_BASE,
                    &format!("StageName_R{}_ja.csv", range.type_name),
                ),
                join_url(
                    ASSET_RESOURCE_BASE,
                    &format!("StageName_{}_ja.csv", range.type_name),
                ),
            ],
        });
    }
    for definition in CHAPTER_STAGES {
        requests.push(DocumentRequest::single(
            &format!("chapter:{}", definition.file_name),
            &join_url(CHAPTER_STAGE_BASE, definition.file_name),
        ));
    }

    let previous_resources = Arc::new(previous_resources);
    let mut tasks = JoinSet::new();
    for request in requests {
        let http = Arc::clone(http);
        let previous_resources = Arc::clone(&previous_resources);
        tasks.spawn(async move { fetch_document(http, previous_resources, request).await });
    }

    let mut documents = HashMap::new();
    while let Some(result) = tasks.join_next().await {
        let document =
            result.map_err(|error| StageDataError::new(format!("data task failed: {error}")))??;
        resources.insert(document.url, document.state.clone());
        documents.insert(document.key, document.state.text);
    }

    let map_names = parse_id_names(required_document(&documents, "map")?, false, "Map_Name.csv")?;
    let sale_names = parse_id_names(
        required_document(&documents, "sale")?,
        true,
        "sale_name.csv",
    )?;
    let mut normal_stages = HashMap::new();
    for range in std::iter::once(StageTypeRange {
        from: 0,
        to: 999,
        type_name: "N".to_owned(),
    })
    .chain(ranges.iter().cloned())
    {
        let key = format!("normal:{}", range.type_name);
        normal_stages.insert(
            range.type_name,
            parse_stage_names(required_document(&documents, &key)?, &key)?,
        );
    }
    let mut chapter_stages = HashMap::new();
    for definition in CHAPTER_STAGES {
        let key = format!("chapter:{}", definition.file_name);
        chapter_stages.insert(
            definition.file_name,
            parse_stage_names(required_document(&documents, &key)?, &key)?,
        );
    }

    let value = Arc::new(build_search_data(
        ranges,
        map_names,
        sale_names,
        normal_stages,
        chapter_stages,
    )?);
    Ok(StageSnapshot {
        value,
        resources,
        validated_at: now,
    })
}

fn required_document<'a>(
    documents: &'a HashMap<String, Arc<str>>,
    key: &str,
) -> Result<&'a str, StageDataError> {
    documents
        .get(key)
        .map(AsRef::as_ref)
        .ok_or_else(|| StageDataError::new(format!("missing downloaded document: {key}")))
}

struct DocumentRequest {
    key: String,
    candidates: Vec<String>,
}

impl DocumentRequest {
    fn single(key: &str, url: &str) -> Self {
        Self {
            key: key.to_owned(),
            candidates: vec![url.to_owned()],
        }
    }
}

struct FetchedDocument {
    key: String,
    url: String,
    state: ResourceState,
}

async fn fetch_document(
    http: Arc<HttpService>,
    previous: Arc<HashMap<String, ResourceState>>,
    request: DocumentRequest,
) -> Result<FetchedDocument, StageDataError> {
    for url in &request.candidates {
        match fetch_resource(&http, url, previous.get(url).cloned()).await {
            Ok(state) => {
                return Ok(FetchedDocument {
                    key: request.key,
                    url: url.clone(),
                    state,
                });
            }
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(error),
        }
    }
    Err(StageDataError::new(format!(
        "all candidates were missing for {}",
        request.key
    )))
}

async fn fetch_resource(
    http: &HttpService,
    url: &str,
    previous: Option<ResourceState>,
) -> Result<ResourceState, StageDataError> {
    let response = http
        .get_conditional_text(
            url,
            previous.as_ref().and_then(|entry| entry.etag.as_deref()),
            previous
                .as_ref()
                .and_then(|entry| entry.last_modified.as_deref()),
        )
        .await
        .map_err(|error| StageDataError::http(url, error))?;
    match response {
        ConditionalTextResponse::NotModified {
            etag,
            last_modified,
        } => {
            let mut current = previous.ok_or_else(|| {
                StageDataError::new(format!("304 response without cached data: {url}"))
            })?;
            if etag.is_some() {
                current.etag = etag;
            }
            if last_modified.is_some() {
                current.last_modified = last_modified;
            }
            Ok(current)
        }
        ConditionalTextResponse::Modified {
            text,
            etag,
            last_modified,
        } => Ok(ResourceState {
            text: Arc::from(text),
            etag,
            last_modified,
        }),
    }
}

fn join_url(base: &str, file_name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), file_name)
}

#[derive(Clone)]
struct StageTypeRange {
    from: usize,
    to: usize,
    type_name: String,
}

type StageNameRows = Vec<Vec<Option<String>>>;

fn parse_stage_types(text: &str) -> Result<Vec<StageTypeRange>, StageDataError> {
    let lines = lines_without_trailing_empty(text);
    if lines.len() < 2 || !lines[0].trim().eq_ignore_ascii_case("from,to,type") {
        return Err(StageDataError::new("invalid stage_type.csv header"));
    }
    if lines.len() - 1 > MAX_STAGE_TYPES {
        return Err(StageDataError::new("stage_type.csv has too many rows"));
    }
    let mut types = BTreeSet::new();
    let mut ranges = Vec::new();
    for (index, line) in lines.iter().skip(1).enumerate() {
        let cells = line.split(',').map(str::trim).collect::<Vec<_>>();
        if cells.len() != 3 {
            return Err(StageDataError::new(format!(
                "stage_type.csv row {} must have 3 columns",
                index + 2
            )));
        }
        let from = parse_usize(cells[0], "stage range start")?;
        let to = parse_usize(cells[1], "stage range end")?;
        let type_name = cells[2];
        let normalized = type_name.to_ascii_lowercase();
        if to < from || to - from > 999 || !valid_type_name(type_name) || !types.insert(normalized)
        {
            return Err(StageDataError::new(format!(
                "stage_type.csv row {} has an invalid range or type",
                index + 2
            )));
        }
        ranges.push(StageTypeRange {
            from,
            to,
            type_name: type_name.to_owned(),
        });
    }
    Ok(ranges)
}

fn valid_type_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn parse_id_names(
    text: &str,
    allow_negative: bool,
    label: &str,
) -> Result<BTreeMap<usize, String>, StageDataError> {
    let lines = lines_without_trailing_empty(text);
    if lines.is_empty() || lines.len() > MAX_NAME_ROWS {
        return Err(StageDataError::new(format!("invalid {label} row count")));
    }
    let mut names = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        let Some((id_text, name_text)) = line.split_once(',') else {
            return Err(StageDataError::new(format!(
                "{label} row {} has no name",
                index + 1
            )));
        };
        if allow_negative && id_text.trim().starts_with('-') {
            continue;
        }
        let id = parse_usize(id_text.trim(), label)?;
        let name = normalized_name(name_text)?;
        if let Some(name) = name
            && names.insert(id, name).is_some()
        {
            return Err(StageDataError::new(format!("duplicate ID in {label}")));
        }
    }
    if names.is_empty() {
        return Err(StageDataError::new(format!("{label} has no names")));
    }
    Ok(names)
}

fn parse_stage_names(text: &str, label: &str) -> Result<StageNameRows, StageDataError> {
    let lines = lines_without_trailing_empty(text);
    if lines.is_empty() || lines.len() > MAX_NAME_ROWS {
        return Err(StageDataError::new(format!("invalid {label} row count")));
    }
    let mut found = false;
    let mut rows = Vec::with_capacity(lines.len());
    for line in lines {
        let cells = line.split(',').collect::<Vec<_>>();
        if cells.len() > MAX_STAGES_PER_ROW {
            return Err(StageDataError::new(format!("{label} row is too wide")));
        }
        let mut row = Vec::with_capacity(cells.len());
        for cell in cells {
            let name = normalized_name(cell)?;
            found |= name.is_some();
            row.push(name);
        }
        rows.push(row);
    }
    if !found {
        return Err(StageDataError::new(format!("{label} has no stage names")));
    }
    Ok(rows)
}

fn normalized_name(value: &str) -> Result<Option<String>, StageDataError> {
    let name = value.trim();
    if name.is_empty() || name == "@" || name == "＠" {
        return Ok(None);
    }
    if name.encode_utf16().count() > MAX_NAME_UTF16 {
        return Err(StageDataError::new("stage name is too long"));
    }
    Ok(Some(name.to_owned()))
}

fn lines_without_trailing_empty(text: &str) -> Vec<&str> {
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

fn parse_usize(value: &str, label: &str) -> Result<usize, StageDataError> {
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return Err(StageDataError::new(format!("invalid integer in {label}")));
    }
    value
        .parse()
        .map_err(|_| StageDataError::new(format!("integer out of range in {label}")))
}

#[derive(Clone, Copy)]
struct ChapterStageDefinition {
    file_name: &'static str,
    raw_map_start: usize,
    jdb_type: &'static str,
}

const CHAPTER_STAGES: &[ChapterStageDefinition] = &[
    ChapterStageDefinition {
        file_name: "StageName0_ja.csv",
        raw_map_start: 3_000,
        jdb_type: "0",
    },
    ChapterStageDefinition {
        file_name: "StageName1_ja.csv",
        raw_map_start: 3_003,
        jdb_type: "1",
    },
    ChapterStageDefinition {
        file_name: "StageName2_ja.csv",
        raw_map_start: 3_006,
        jdb_type: "2",
    },
    ChapterStageDefinition {
        file_name: "StageName0Z_ja.csv",
        raw_map_start: 20_000,
        jdb_type: "0Z",
    },
    ChapterStageDefinition {
        file_name: "StageName1Z_ja.csv",
        raw_map_start: 21_000,
        jdb_type: "1Z",
    },
    ChapterStageDefinition {
        file_name: "StageName2Z_ja.csv",
        raw_map_start: 22_000,
        jdb_type: "2Z",
    },
];

fn build_search_data(
    configured_ranges: Vec<StageTypeRange>,
    map_names: BTreeMap<usize, String>,
    sale_names: BTreeMap<usize, String>,
    normal_stages: HashMap<String, StageNameRows>,
    chapter_stages: HashMap<&'static str, StageNameRows>,
) -> Result<StageSearchData, StageDataError> {
    let ranges = all_ranges(configured_ranges)?;
    let mut maps = Vec::new();
    for (raw_map_id, map_name) in map_names {
        let route = find_route(raw_map_id, &ranges)
            .ok_or_else(|| StageDataError::new(format!("map ID {raw_map_id} has no JDB route")))?;
        let alias = (raw_map_id >= 1_000)
            .then(|| sale_names.get(&raw_map_id))
            .flatten();
        let old_name = map_name.contains("(旧)") || map_name.contains("（旧）");
        let display_name = if old_name {
            map_name.clone()
        } else {
            alias.cloned().unwrap_or_else(|| map_name.clone())
        };
        let mut search_names = vec![map_name];
        if let Some(alias) = alias
            && !search_names.contains(alias)
        {
            search_names.push(alias.clone());
        }
        maps.push(Arc::new(StageEntry {
            kind: StageKind::Map { search_names },
            raw_map_id,
            display_id: route.display_id,
            display_name,
            jdb_type: route.jdb_type,
            jdb_map: route.jdb_map,
            display_type: route.display_type,
            display_map: route.display_map,
        }));
    }

    let mut stages = Vec::new();
    for range in &ranges {
        let rows = normal_stages.get(&range.type_name).ok_or_else(|| {
            StageDataError::new(format!("missing stage names for {}", range.type_name))
        })?;
        if rows.len().saturating_sub(1) > range.to - range.from {
            return Err(StageDataError::new(format!(
                "stage names exceed the range for {}",
                range.type_name
            )));
        }
        append_stages(
            &mut stages,
            rows,
            range.from,
            &range.type_name,
            Some(&range.type_name),
        );
    }
    for definition in CHAPTER_STAGES {
        let rows = chapter_stages
            .get(definition.file_name)
            .ok_or_else(|| StageDataError::new(format!("missing {}", definition.file_name)))?;
        if rows.len() != 3 {
            return Err(StageDataError::new(format!(
                "{} must contain 3 rows",
                definition.file_name
            )));
        }
        append_stages(
            &mut stages,
            rows,
            definition.raw_map_start,
            definition.jdb_type,
            None,
        );
    }
    stages.sort_by_key(|entry| (entry.raw_map_id, entry.stage_index().unwrap_or(0)));

    let mut id_index = HashMap::new();
    for entry in maps.iter().chain(&stages) {
        let stage_index = entry.stage_index();
        insert_id(
            &mut id_index,
            raw_id_key(entry.raw_map_id, stage_index),
            entry,
        )?;
        if let (Some(display_type), Some(display_map)) = (&entry.display_type, entry.display_map) {
            insert_id(
                &mut id_index,
                type_id_key(display_type, display_map, stage_index),
                entry,
            )?;
        }
    }
    let mut display_types = maps
        .iter()
        .chain(&stages)
        .filter_map(|entry| entry.display_type.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    display_types.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    Ok(StageSearchData {
        maps,
        stages,
        display_types,
        id_index,
    })
}

fn all_ranges(mut configured: Vec<StageTypeRange>) -> Result<Vec<StageTypeRange>, StageDataError> {
    configured.push(StageTypeRange {
        from: 0,
        to: 999,
        type_name: "N".to_owned(),
    });
    let mut types = BTreeSet::new();
    for range in &configured {
        if !types.insert(range.type_name.to_ascii_lowercase()) {
            return Err(StageDataError::new("duplicate stage type"));
        }
    }
    configured.sort_by_key(|range| range.from);
    if configured.windows(2).any(|pair| pair[1].from <= pair[0].to) {
        return Err(StageDataError::new("stage type ranges overlap"));
    }
    let reserved = CHAPTER_STAGES
        .iter()
        .flat_map(|definition| definition.raw_map_start..=definition.raw_map_start + 2)
        .chain([23_000, 38_000]);
    if reserved.into_iter().any(|id| {
        configured
            .iter()
            .any(|range| id >= range.from && id <= range.to)
    }) {
        return Err(StageDataError::new(
            "stage type range overlaps a reserved map ID",
        ));
    }
    Ok(configured)
}

struct StageRoute {
    jdb_type: String,
    jdb_map: usize,
    display_id: String,
    display_type: Option<String>,
    display_map: Option<usize>,
}

fn find_route(raw_map_id: usize, ranges: &[StageTypeRange]) -> Option<StageRoute> {
    if let Some(range) = ranges
        .iter()
        .find(|range| raw_map_id >= range.from && raw_map_id <= range.to)
    {
        let map = raw_map_id - range.from;
        return Some(StageRoute {
            jdb_type: range.type_name.clone(),
            jdb_map: map,
            display_id: format!("{}{}", range.type_name, pad_id(map)),
            display_type: Some(range.type_name.clone()),
            display_map: Some(map),
        });
    }
    if let Some(definition) = CHAPTER_STAGES.iter().find(|definition| {
        raw_map_id >= definition.raw_map_start && raw_map_id <= definition.raw_map_start + 2
    }) {
        return Some(StageRoute {
            jdb_type: definition.jdb_type.to_owned(),
            jdb_map: raw_map_id - definition.raw_map_start,
            display_id: raw_map_id.to_string(),
            display_type: None,
            display_map: None,
        });
    }
    match raw_map_id {
        23_000 => Some(StageRoute {
            jdb_type: "2_Inv".to_owned(),
            jdb_map: 0,
            display_id: "2_Inv000".to_owned(),
            display_type: Some("2_Inv".to_owned()),
            display_map: Some(0),
        }),
        38_000 => Some(StageRoute {
            jdb_type: "2Z_Inv".to_owned(),
            jdb_map: 0,
            display_id: "2Z_Inv000".to_owned(),
            display_type: Some("2Z_Inv".to_owned()),
            display_map: Some(0),
        }),
        _ => None,
    }
}

fn append_stages(
    target: &mut Vec<Arc<StageEntry>>,
    rows: &StageNameRows,
    raw_map_start: usize,
    jdb_type: &str,
    display_type: Option<&str>,
) {
    for (map, row) in rows.iter().enumerate() {
        for (stage_index, display_name) in row.iter().enumerate() {
            let Some(display_name) = display_name else {
                continue;
            };
            let raw_map_id = raw_map_start + map;
            let map_id = display_type
                .map(|value| format!("{value}{}", pad_id(map)))
                .unwrap_or_else(|| raw_map_id.to_string());
            target.push(Arc::new(StageEntry {
                kind: StageKind::Stage { stage_index },
                raw_map_id,
                display_id: format!("{map_id}-{}", pad_id(stage_index)),
                display_name: display_name.clone(),
                jdb_type: jdb_type.to_owned(),
                jdb_map: map,
                display_type: display_type.map(str::to_owned),
                display_map: display_type.map(|_| map),
            }));
        }
    }
}

fn pad_id(value: usize) -> String {
    format!("{value:03}")
}

fn raw_id_key(raw_map_id: usize, stage_index: Option<usize>) -> String {
    stage_index.map_or_else(
        || format!("raw:{raw_map_id}"),
        |stage| format!("raw:{raw_map_id}:stage:{stage}"),
    )
}

fn type_id_key(type_name: &str, map: usize, stage_index: Option<usize>) -> String {
    let base = format!("type:{}:{map}", type_name.to_ascii_lowercase());
    stage_index.map_or(base.clone(), |stage| format!("{base}:stage:{stage}"))
}

fn insert_id(
    index: &mut HashMap<String, Arc<StageEntry>>,
    key: String,
    entry: &Arc<StageEntry>,
) -> Result<(), StageDataError> {
    if index.insert(key.clone(), Arc::clone(entry)).is_some() {
        return Err(StageDataError::new(format!(
            "duplicate stage search ID: {key}"
        )));
    }
    Ok(())
}

fn parse_id_key(query: &str, display_types: &[String]) -> Option<String> {
    if query.is_empty()
        || !query
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
    {
        return None;
    }
    let (base, stage_index) = match query.split_once('-') {
        Some((base, stage)) if !stage.contains('-') => (base, Some(stage.parse().ok()?)),
        Some(_) => return None,
        None => (query, None),
    };
    if base.chars().all(|character| character.is_ascii_digit()) {
        return Some(raw_id_key(base.parse().ok()?, stage_index));
    }
    let lower = base.to_ascii_lowercase();
    for type_name in display_types {
        let normalized_type = type_name.to_ascii_lowercase();
        if !lower.starts_with(&normalized_type) {
            continue;
        }
        let map = base.get(type_name.len()..)?.parse().ok()?;
        return Some(type_id_key(type_name, map, stage_index));
    }
    None
}

#[derive(Debug)]
pub(super) struct StageDataError {
    reason: String,
    not_found: bool,
}

impl StageDataError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            not_found: false,
        }
    }

    fn http(url: &str, error: HttpServiceError) -> Self {
        Self {
            not_found: matches!(error, HttpServiceError::Status(StatusCode::NOT_FOUND)),
            reason: format!("{url}: {error}"),
        }
    }

    fn is_not_found(&self) -> bool {
        self.not_found
    }
}

impl Display for StageDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)
    }
}
