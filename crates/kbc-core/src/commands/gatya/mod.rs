//! 外部Event dataを使うgatya Command。

mod data_source;
pub(in crate::commands) mod model;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::services::Clock;

use super::common::output::{push_chunked, push_message};
use super::common::schedule::{format_jst_full, format_jst_short, parse_header_date};
use data_source::GatyaDataSource;
use model::{
    GachaBlock, GachaEntry, GachaJson, GachaLookupData, GachaMode, GachaScheduleData,
    ItemScheduleEntry, MappingMaps, ModeMaps, NameMaps,
};

pub(super) use data_source::GatyaDataSource as RegisteredGatyaDataSource;

pub(super) struct GatyaCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: GatyaDataSource,
    clock: Arc<dyn Clock>,
}

impl GatyaCommand {
    pub(super) fn new(help: &str, data_source: GatyaDataSource, clock: Arc<dyn Clock>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "gatya".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            data_source,
            clock,
        }
    }

    async fn run(
        &self,
        channel_id: &str,
        arguments: &[String],
    ) -> Result<CommandOutput, crate::command::CommandExecutionError> {
        if arguments
            .first()
            .is_some_and(|argument| argument.eq_ignore_ascii_case("help"))
        {
            return single_message(channel_id, self.help.clone());
        }

        match parse_request(arguments) {
            GatyaRequest::Schedule { mode } => match self.data_source.fetch_schedule_data().await {
                Ok(data) => {
                    let content = format_schedule(&data, self.clock.now(), mode)
                        .unwrap_or_else(|| "開催中・近日予定のガチャはありません".to_owned());
                    chunked_message(channel_id, &content, "")
                }
                Err(error) => data_error(channel_id, error),
            },
            GatyaRequest::Detail { mode, target } => {
                match self.data_source.fetch_lookup_data().await {
                    Ok(data) => format_details(channel_id, mode, target, &data),
                    Err(error) => data_error(channel_id, error),
                }
            }
            GatyaRequest::Search { mode, query } => {
                match self.data_source.fetch_lookup_data().await {
                    Ok(data) => format_search(channel_id, mode, &query, &data),
                    Err(error) => data_error(channel_id, error),
                }
            }
            GatyaRequest::Json { mode, target } => {
                format_json_or_raw(channel_id, mode, target, false, &self.data_source).await
            }
            GatyaRequest::Raw { mode, target } => {
                format_json_or_raw(channel_id, mode, target, true, &self.data_source).await
            }
        }
    }
}

impl Command for GatyaCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(context.channel_id(), &arguments).await })
    }
}

#[derive(Clone, Copy)]
enum GachaTarget {
    Gacha(i64),
    Series(i64),
}

enum GatyaRequest {
    Schedule {
        mode: Option<GachaMode>,
    },
    Detail {
        mode: Option<GachaMode>,
        target: GachaTarget,
    },
    Json {
        mode: Option<GachaMode>,
        target: GachaTarget,
    },
    Raw {
        mode: Option<GachaMode>,
        target: GachaTarget,
    },
    Search {
        mode: Option<GachaMode>,
        query: String,
    },
}

fn parse_request(arguments: &[String]) -> GatyaRequest {
    let (mode, rest) = match arguments.first().and_then(|value| parse_mode(value)) {
        Some(mode) => (Some(mode), &arguments[1..]),
        None => (None, arguments),
    };
    if rest.is_empty() {
        return GatyaRequest::Schedule { mode };
    }
    if rest.len() == 2
        && let Some(target) = parse_target(&rest[0])
    {
        if rest[1].eq_ignore_ascii_case("j") || rest[1].eq_ignore_ascii_case("json") {
            return GatyaRequest::Json { mode, target };
        }
        if rest[1].eq_ignore_ascii_case("r") || rest[1].eq_ignore_ascii_case("raw") {
            return GatyaRequest::Raw { mode, target };
        }
    }
    if rest.len() == 1
        && let Some(target) = parse_target(&rest[0])
    {
        return GatyaRequest::Detail { mode, target };
    }
    GatyaRequest::Search {
        mode,
        query: rest.join(" ").trim().to_owned(),
    }
}

fn parse_mode(value: &str) -> Option<GachaMode> {
    match value.to_ascii_uppercase().as_str() {
        "R" => Some(GachaMode::Rare),
        "E" => Some(GachaMode::Event),
        "N" => Some(GachaMode::Normal),
        _ => None,
    }
}

fn parse_target(value: &str) -> Option<GachaTarget> {
    if let Ok(id) = value.parse() {
        return Some(GachaTarget::Gacha(id));
    }
    let series = value.strip_prefix(['s', 'S'])?;
    if series.is_empty() || !series.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    series.parse().ok().map(GachaTarget::Series)
}

async fn format_json_or_raw(
    channel_id: &str,
    mode: Option<GachaMode>,
    target: GachaTarget,
    is_raw: bool,
    data_source: &GatyaDataSource,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let blocks: Vec<GachaBlock> = match target {
        GachaTarget::Series(series_id) => match data_source.fetch_json_with_mappings().await {
            Ok(data) => find_series_blocks(&data.gacha, series_id, mode, &data.series_mappings)
                .into_iter()
                .cloned()
                .collect(),
            Err(error) => return data_error(channel_id, error),
        },
        GachaTarget::Gacha(id) => match data_source.fetch_gacha_json().await {
            Ok(data) => find_gacha_blocks(&data, id, mode)
                .into_iter()
                .cloned()
                .collect(),
            Err(error) => return data_error(channel_id, error),
        },
    };
    if blocks.is_empty() {
        return single_message(channel_id, target_error(target));
    }

    let mut output = CommandOutput::new();
    for block in blocks {
        if is_raw {
            match &block.raw {
                Some(raw) => push_chunked(&mut output, channel_id, &raw.replace('\t', "    "), "")?,
                None => push_message(
                    &mut output,
                    channel_id,
                    format!(
                        "❌ (startDate: {}) に raw データがありません",
                        block.header.schedule.start_date
                    ),
                )?,
            }
        } else {
            match serde_json::to_string_pretty(&block) {
                Ok(json) => push_chunked(&mut output, channel_id, &json, "json")?,
                Err(error) => return data_error(channel_id, error),
            }
        }
    }
    Ok(output)
}

fn format_details(
    channel_id: &str,
    mode: Option<GachaMode>,
    target: GachaTarget,
    data: &GachaLookupData,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    if let GachaTarget::Series(series_id) = target {
        let names = merge_series_names(&data.series_names, &data.short_series_names);
        let summaries = find_series_summaries(series_id, mode, &names, &data.series_mappings);
        if summaries.is_empty() {
            return single_message(channel_id, target_error(target));
        }
        return chunked_message(channel_id, &format_series_summaries(&summaries), "");
    }

    let GachaTarget::Gacha(id) = target else {
        unreachable!();
    };
    let blocks = find_gacha_blocks(&data.gacha, id, mode);
    if blocks.is_empty() {
        return single_message(channel_id, target_error(target));
    }
    let mut output = CommandOutput::new();
    for block in blocks {
        if let Some(entry) = block.gachas.iter().find(|entry| entry.id == id) {
            push_chunked(
                &mut output,
                channel_id,
                &format_gacha_detail(block, entry, data),
                "",
            )?;
        }
    }
    Ok(output)
}

fn format_search(
    channel_id: &str,
    mode: Option<GachaMode>,
    query: &str,
    data: &GachaLookupData,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let names = merge_series_names(&data.series_names, &data.short_series_names);
    let normalized = query.to_lowercase();
    let mut summaries = Vec::new();
    for candidate_mode in modes(mode) {
        let mut matches = names
            .get(candidate_mode)
            .iter()
            .filter(|(_, name)| name.to_lowercase().contains(&normalized))
            .collect::<Vec<_>>();
        matches.sort_by_key(|(series_id, _)| **series_id);
        for (series_id, name) in matches {
            let gacha_ids = series_member_ids(data.series_mappings.get(candidate_mode), *series_id);
            if !gacha_ids.is_empty() {
                summaries.push(SeriesSummary {
                    mode: candidate_mode,
                    series_id: *series_id,
                    name: name.clone(),
                    gacha_ids,
                });
            }
        }
    }
    if summaries.is_empty() {
        return single_message(
            channel_id,
            format!("❌ スケジュール内に `{query}` は見つかりませんでした"),
        );
    }
    chunked_message(channel_id, &format_series_summaries(&summaries), "")
}

fn format_schedule(
    data: &GachaScheduleData,
    now: DateTime<Utc>,
    mode: Option<GachaMode>,
) -> Option<String> {
    enum Event<'a> {
        Gacha {
            block: &'a GachaBlock,
            start: DateTime<Utc>,
            end: DateTime<Utc>,
        },
        Item {
            entry: &'a ItemScheduleEntry,
            start: DateTime<Utc>,
            end: DateTime<Utc>,
        },
    }
    impl Event<'_> {
        fn start(&self) -> DateTime<Utc> {
            match self {
                Self::Gacha { start, .. } | Self::Item { start, .. } => *start,
            }
        }

        fn end(&self) -> DateTime<Utc> {
            match self {
                Self::Gacha { end, .. } | Self::Item { end, .. } => *end,
            }
        }

        fn date_key(&self, active: bool) -> &str {
            match self {
                Self::Gacha { block, .. } => {
                    if active {
                        &block.header.schedule.end_date
                    } else {
                        &block.header.schedule.start_date
                    }
                }
                Self::Item { entry, .. } => {
                    if active {
                        &entry.header.end_date
                    } else {
                        &entry.header.start_date
                    }
                }
            }
        }
    }

    let mut events = data
        .gacha
        .data
        .iter()
        .filter(|block| {
            !is_permanent(block)
                && mode_matches_type(mode, block.header.gacha_type)
                && gacha_end(block) > now
        })
        .map(|block| Event::Gacha {
            block,
            start: gacha_start(block),
            end: gacha_end(block),
        })
        .collect::<Vec<_>>();
    if !matches!(mode, Some(GachaMode::Event | GachaMode::Normal)) {
        events.extend(
            data.item
                .data
                .iter()
                .filter(|entry| matches!(entry.gift.gift_type, 301 | 302))
                .filter(|entry| {
                    let start = item_start(entry);
                    if entry.header.end_date == "20300101" {
                        start > now
                    } else {
                        item_end(entry) > now
                    }
                })
                .map(|entry| Event::Item {
                    entry,
                    start: item_start(entry),
                    end: item_end(entry),
                }),
        );
    }
    events.sort_by_key(Event::start);

    struct ScheduleSeries {
        series_id: Option<i64>,
        name: String,
        entries: Vec<GachaEntry>,
        gacha_types: HashSet<i64>,
    }
    struct Group {
        key: String,
        active: bool,
        period: String,
        series: Vec<(String, ScheduleSeries)>,
        gift_items: HashMap<i64, String>,
    }

    let mut groups: Vec<Group> = Vec::new();
    for event in events {
        let start = event.start();
        let end = event.end();
        let active = now >= start && now < end;
        let key = format!(
            "{}:{}",
            if active { "active" } else { "upcoming" },
            event.date_key(active).trim()
        );
        let group_index = groups.iter().position(|group| group.key == key);
        let index = group_index.unwrap_or_else(|| {
            groups.push(Group {
                key,
                active,
                period: if active {
                    format!("~{}", format_jst_short(end))
                } else {
                    format!("{}~", format_jst_short(start))
                },
                series: Vec::new(),
                gift_items: HashMap::new(),
            });
            groups.len() - 1
        });
        let group = &mut groups[index];

        match event {
            Event::Item { entry, .. } => {
                group.gift_items.insert(
                    entry.gift.gift_type,
                    data.sale_names
                        .get(&entry.gift.gift_type)
                        .cloned()
                        .unwrap_or_else(|| "不明".to_owned()),
                );
            }
            Event::Gacha { block, .. } => {
                let block_mode = mode_for_type(block.header.gacha_type);
                for entry in &block.gachas {
                    if entry.id < 0 {
                        continue;
                    }
                    let series_id =
                        series_id(&data.series_mappings, block.header.gacha_type, entry.id);
                    let series_key = series_id.map_or_else(
                        || format!("{}:unknown:{}", mode_label(block_mode), entry.id),
                        |id| format!("{}:{id}", mode_label(block_mode)),
                    );
                    let series_index = group.series.iter().position(|(key, _)| *key == series_key);
                    let series_position = series_index.unwrap_or_else(|| {
                        let name = series_id
                            .and_then(|id| data.short_series_names.get(block_mode).get(&id))
                            .cloned()
                            .unwrap_or_else(|| "不明".to_owned());
                        group.series.push((
                            series_key,
                            ScheduleSeries {
                                series_id,
                                name,
                                entries: Vec::new(),
                                gacha_types: HashSet::new(),
                            },
                        ));
                        group.series.len() - 1
                    });
                    let series = &mut group.series[series_position].1;
                    if let Some(existing) =
                        series.entries.iter_mut().find(|item| item.id == entry.id)
                    {
                        *existing = entry.clone();
                    } else {
                        series.entries.push(entry.clone());
                    }
                    series.gacha_types.insert(block.header.gacha_type);
                }
            }
        }
    }

    let mut lines = Vec::new();
    for group in groups {
        lines.push(format!(
            "{}[{}]",
            if group.active { "🟢 " } else { "" },
            group.period
        ));
        let mut gifts = group.gift_items.into_iter().collect::<Vec<_>>();
        gifts.sort_by_key(|(gift_type, _)| *gift_type);
        lines.extend(
            gifts
                .into_iter()
                .map(|(gift_type, name)| format!("    {gift_type} {name}")),
        );
        for (_, mut series) in group.series {
            series.entries.sort_by_key(|entry| entry.id);
            let label = series
                .series_id
                .map_or_else(|| "s?".to_owned(), |id| format!("s{id}"));
            let tag = if series.gacha_types.len() == 1 {
                type_tag(*series.gacha_types.iter().next().expect("one type"))
            } else {
                ""
            };
            for entry in series.entries {
                lines.push(format!(
                    "    {} {label} {}{}{}",
                    entry.id,
                    series.name,
                    entry_labels(&entry),
                    tag
                ));
            }
        }
    }
    (!lines.is_empty())
        .then(|| format!("ガチャスケジュール [開催中＆予定]\n\n{}", lines.join("\n")))
}

fn find_gacha_blocks(gacha: &GachaJson, id: i64, mode: Option<GachaMode>) -> Vec<&GachaBlock> {
    gacha
        .data
        .iter()
        .filter(|block| {
            mode_matches_type(mode, block.header.gacha_type)
                && block.gachas.iter().any(|entry| entry.id == id)
        })
        .collect()
}

fn find_series_blocks<'a>(
    gacha: &'a GachaJson,
    target_series_id: i64,
    mode: Option<GachaMode>,
    mappings: &MappingMaps,
) -> Vec<&'a GachaBlock> {
    gacha
        .data
        .iter()
        .filter(|block| {
            mode_matches_type(mode, block.header.gacha_type)
                && block.gachas.iter().any(|entry| {
                    series_id(mappings, block.header.gacha_type, entry.id) == Some(target_series_id)
                })
        })
        .collect()
}

fn format_gacha_detail(block: &GachaBlock, entry: &GachaEntry, data: &GachaLookupData) -> String {
    let mode = mode_for_type(block.header.gacha_type);
    let name = data
        .gacha_names
        .get(mode)
        .get(&entry.id)
        .map(String::as_str)
        .unwrap_or("不明");
    let series_label = series_id(&data.series_mappings, block.header.gacha_type, entry.id)
        .map_or_else(|| "s?".to_owned(), |id| format!("s{id}"));
    let end_text = if is_permanent(block) {
        "常設".to_owned()
    } else {
        format_jst_full(gacha_end(block))
    };
    let mut lines = vec![
        format!(
            "{} ～ {}  ver.{}～{}",
            format_jst_full(gacha_start(block)),
            end_text,
            block.header.schedule.min_version,
            block.header.schedule.max_version
        ),
        format!(
            "{} {series_label} {name}{}{}",
            entry.id,
            entry_labels(entry),
            type_tag(block.header.gacha_type)
        ),
    ];
    let rates = format_rates(entry);
    if !rates.is_empty() {
        lines.push(format!("レート: {rates}"));
    }
    if let Some(message) = entry
        .message
        .as_deref()
        .filter(|message| !message.is_empty())
    {
        lines.push(format!("メッセージ: {message}"));
    }
    lines.join("\n")
}

fn format_rates(entry: &GachaEntry) -> String {
    [
        ("ノーマル", &entry.rates.normal),
        ("レア", &entry.rates.rare),
        ("超激レア", &entry.rates.uber_rare),
        ("伝説レア", &entry.rates.legend_rare),
    ]
    .into_iter()
    .filter(|(_, value)| value.as_f64() != Some(0.0))
    .map(|(label, value)| format!("{label} {value}"))
    .collect::<Vec<_>>()
    .join(", ")
}

struct SeriesSummary {
    mode: GachaMode,
    series_id: i64,
    name: String,
    gacha_ids: Vec<i64>,
}

fn find_series_summaries(
    target_series_id: i64,
    mode: Option<GachaMode>,
    names: &NameMaps,
    mappings: &MappingMaps,
) -> Vec<SeriesSummary> {
    modes(mode)
        .into_iter()
        .filter_map(|candidate_mode| {
            let gacha_ids = series_member_ids(mappings.get(candidate_mode), target_series_id);
            (!gacha_ids.is_empty()).then(|| SeriesSummary {
                mode: candidate_mode,
                series_id: target_series_id,
                name: names
                    .get(candidate_mode)
                    .get(&target_series_id)
                    .cloned()
                    .unwrap_or_else(|| "不明".to_owned()),
                gacha_ids,
            })
        })
        .collect()
}

fn format_series_summaries(summaries: &[SeriesSummary]) -> String {
    summaries
        .iter()
        .map(|summary| {
            let tag = type_tag(type_for_mode(summary.mode));
            summary
                .gacha_ids
                .iter()
                .map(|id| format!("{id} s{} {}{tag}", summary.series_id, summary.name))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn merge_series_names(full: &NameMaps, short: &NameMaps) -> NameMaps {
    ModeMaps {
        rare: merge_name_map(&full.rare, &short.rare),
        event: merge_name_map(&full.event, &short.event),
        normal: merge_name_map(&full.normal, &short.normal),
    }
}

fn merge_name_map(
    preferred: &HashMap<i64, String>,
    fallback: &HashMap<i64, String>,
) -> HashMap<i64, String> {
    let mut names = fallback.clone();
    names.extend(preferred.iter().map(|(id, name)| (*id, name.clone())));
    names
}

fn series_member_ids(mapping: &HashMap<i64, i64>, target_series_id: i64) -> Vec<i64> {
    let mut ids = mapping
        .iter()
        .filter_map(|(gacha_id, series_id)| (*series_id == target_series_id).then_some(*gacha_id))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

pub(in crate::commands) fn series_id(
    mappings: &MappingMaps,
    gacha_type: i64,
    gacha_id: i64,
) -> Option<i64> {
    mappings
        .get(mode_for_type(gacha_type))
        .get(&gacha_id)
        .copied()
}

pub(in crate::commands) fn entry_labels(entry: &GachaEntry) -> String {
    let mut labels = String::new();
    if entry.guaranteed {
        labels.push_str("【確定】");
    }
    labels.push_str(match entry.flags {
        4 => "【step up】",
        20_600 => "＋福引＆かけら",
        16_384 => "＋かけら",
        4_216 => "＋福引",
        _ => "",
    });
    labels
}

pub(in crate::commands) fn type_tag(gacha_type: i64) -> &'static str {
    match gacha_type {
        4 => " <イベント>",
        0 => " <ノーマル>",
        _ => "",
    }
}

pub(in crate::commands) fn mode_for_type(gacha_type: i64) -> GachaMode {
    match gacha_type {
        1 => GachaMode::Rare,
        4 => GachaMode::Event,
        _ => GachaMode::Normal,
    }
}

fn type_for_mode(mode: GachaMode) -> i64 {
    match mode {
        GachaMode::Rare => 1,
        GachaMode::Event => 4,
        GachaMode::Normal => 0,
    }
}

fn mode_label(mode: GachaMode) -> &'static str {
    match mode {
        GachaMode::Rare => "R",
        GachaMode::Event => "E",
        GachaMode::Normal => "N",
    }
}

fn mode_matches_type(mode: Option<GachaMode>, gacha_type: i64) -> bool {
    mode.is_none_or(|mode| type_for_mode(mode) == gacha_type)
}

fn modes(mode: Option<GachaMode>) -> Vec<GachaMode> {
    mode.map_or_else(
        || vec![GachaMode::Rare, GachaMode::Event, GachaMode::Normal],
        |mode| vec![mode],
    )
}

fn target_error(target: GachaTarget) -> String {
    match target {
        GachaTarget::Gacha(id) => format!("❌ ID `{id}` はガチャjsonに含まれていません"),
        GachaTarget::Series(id) => {
            format!("❌ seriesID `s{id}` はガチャデータに含まれていません")
        }
    }
}

fn gacha_start(block: &GachaBlock) -> DateTime<Utc> {
    parse_header_date(
        &block.header.schedule.start_date,
        &block.header.schedule.start_time,
    )
    .expect("gatya headers are validated when loaded")
}

fn gacha_end(block: &GachaBlock) -> DateTime<Utc> {
    parse_header_date(
        &block.header.schedule.end_date,
        &block.header.schedule.end_time,
    )
    .expect("gatya headers are validated when loaded")
}

fn item_start(entry: &ItemScheduleEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.start_date, &entry.header.start_time)
        .expect("item headers are validated when loaded")
}

fn item_end(entry: &ItemScheduleEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.end_date, &entry.header.end_time)
        .expect("item headers are validated when loaded")
}

fn is_permanent(block: &GachaBlock) -> bool {
    block.header.schedule.end_date == "20300101"
}

fn single_message(
    channel_id: &str,
    content: impl Into<String>,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    Ok(CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    }))
}

fn chunked_message(
    channel_id: &str,
    content: &str,
    language: &str,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let mut output = CommandOutput::new();
    push_chunked(&mut output, channel_id, content, language)?;
    Ok(output)
}

fn data_error<E: std::fmt::Display>(
    channel_id: &str,
    error: E,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    eprintln!("Gatya data retrieval failed: {error}");
    single_message(channel_id, "❌ データ取得に失敗しました")
}

#[cfg(test)]
mod tests {
    use super::{GachaMode, GachaTarget, GatyaRequest, parse_request};

    #[test]
    fn parses_supported_gatya_requests() {
        assert!(matches!(
            parse_request(&[]),
            GatyaRequest::Schedule { mode: None }
        ));
        assert!(matches!(
            parse_request(&["E".to_owned(), "12".to_owned()]),
            GatyaRequest::Detail {
                mode: Some(GachaMode::Event),
                target: GachaTarget::Gacha(12)
            }
        ));
        assert!(matches!(
            parse_request(&["R".to_owned(), "s7".to_owned(), "json".to_owned()]),
            GatyaRequest::Json {
                mode: Some(GachaMode::Rare),
                target: GachaTarget::Series(7)
            }
        ));
    }
}
