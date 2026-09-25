//! 追加・変更されたScheduleをDiscord向けMessageへ整形する。

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::commands::common::schedule::{
    ScheduleHeader, format_duration, format_jst_short, parse_header_date,
};
use crate::commands::gatya::model::{GachaEntry, GachaScheduleData};
use crate::commands::gatya::{entry_labels, mode_for_type, series_id, type_tag};
use crate::commands::item::model::{ItemDisplayData, ItemEntry};
use crate::commands::item::schedule_name;
use crate::commands::sale::model::SaleDisplayData;
use crate::commands::sale::{
    is_mission_id, list_stage_ids, stage_name, stage_name_preserving_breaks,
};

use super::diff::ScheduleChanges;

const MISSION_DISPLAY_LIMIT: usize = 5;
const MESSAGE_LIMIT: usize = 2_000;

#[derive(Default)]
pub(super) struct AddedScheduleData {
    pub(super) gatya: Option<GachaScheduleData>,
    pub(super) sale: Option<SaleDisplayData>,
    pub(super) item: Option<ItemDisplayData>,
    pub(super) changes: ScheduleChanges,
}

struct Row {
    header: ScheduleHeader,
    label: String,
    id: Option<i64>,
}

struct CodeGroup {
    key: String,
    title: String,
    labels: Vec<String>,
}

pub(super) fn format_added_schedules(
    data: &AddedScheduleData,
    now: DateTime<Utc>,
    history_url: &str,
) -> Result<Vec<String>, String> {
    let mut gatya_rows = Vec::new();
    let mut sale_rows = Vec::new();
    let mut item_rows = Vec::new();
    let mut mission_rows = Vec::new();
    if let Some(gatya) = &data.gatya {
        for block in &gatya.gacha.data {
            for entry in &block.gachas {
                if entry.id >= 0 {
                    gatya_rows.push(Row {
                        header: block.header.schedule.clone(),
                        label: gacha_label(entry, block.header.gacha_type, gatya),
                        id: None,
                    });
                }
            }
        }
    }
    if let Some(sale) = &data.sale {
        for entry in &sale.sale.data {
            let duration = if is_permanent(&entry.header) {
                String::new()
            } else {
                format!(
                    " {}",
                    format_duration(
                        parse_header_date(&entry.header.start_date, &entry.header.start_time)?,
                        parse_header_date(&entry.header.end_date, &entry.header.end_time)?,
                    )
                )
            };
            for id in list_stage_ids(entry, &sale.card_setting_stage_ids) {
                sale_rows.push(Row {
                    header: entry.header.clone(),
                    label: format!("{id} {}{duration}", stage_name(id, sale)),
                    id: None,
                });
            }
            for id in entry
                .stage_ids
                .iter()
                .copied()
                .filter(|id| is_mission_id(*id))
            {
                mission_rows.push(Row {
                    header: entry.header.clone(),
                    label: format!("{id} {}{duration}", stage_name_preserving_breaks(id, sale)),
                    id: Some(id),
                });
            }
        }
    }
    if let Some(item) = &data.item {
        for entry in &item.item.data {
            item_rows.push(Row {
                header: entry.header.clone(),
                label: item_label(entry, item),
                id: None,
            });
        }
    }

    let mut output = Vec::new();
    output.extend(format_section("gatya", gatya_rows, now)?);
    output.extend(format_section("sale", sale_rows, now)?);
    output.extend(format_section("item", item_rows, now)?);
    output.extend(format_section("mission", mission_rows, now)?);
    output.extend(format_changes(data, now)?);
    output.push(format!("**関連サイト**\n<{history_url}>"));
    Ok(output)
}

fn gacha_label(entry: &GachaEntry, gacha_type: i64, data: &GachaScheduleData) -> String {
    let mode = mode_for_type(gacha_type);
    let series = series_id(&data.series_mappings, gacha_type, entry.id);
    let name = series
        .and_then(|id| data.short_series_names.get(mode).get(&id))
        .map(String::as_str)
        .unwrap_or("不明");
    format!(
        "{} s{} {}{}{}",
        entry.id,
        series.map_or_else(|| "?".to_owned(), |value| value.to_string()),
        name,
        entry_labels(entry),
        type_tag(gacha_type)
    )
}

fn item_label(entry: &ItemEntry, data: &ItemDisplayData) -> String {
    let amount = if entry.gift.gift_amount > 0 {
        format!(" ×{}", entry.gift.gift_amount)
    } else {
        String::new()
    };
    format!(
        "{} {}{amount}",
        entry.gift.gift_type,
        schedule_name(entry, data)
    )
}

fn format_section(name: &str, rows: Vec<Row>, now: DateTime<Utc>) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut visible = rows
        .into_iter()
        .filter(|row| seen.insert(row_key(row)) && is_visible(&row.header, now).unwrap_or(false))
        .collect::<Vec<_>>();
    visible
        .sort_by_key(|row| parse_header_date(&row.header.start_date, &row.header.start_time).ok());
    let shown = if name == "mission" {
        visible.len().min(MISSION_DISPLAY_LIMIT)
    } else {
        visible.len()
    };
    let mut groups: Vec<CodeGroup> = Vec::new();
    for row in &visible[..shown] {
        let start = parse_header_date(&row.header.start_date, &row.header.start_time)?;
        let active = start <= now;
        let permanent = is_permanent(&row.header);
        let use_end = active && !permanent;
        let date = if use_end {
            parse_header_date(&row.header.end_date, &row.header.end_time)?
        } else {
            start
        };
        let key = format!(
            "{permanent}:{active}:{}",
            if use_end {
                &row.header.end_date
            } else {
                &row.header.start_date
            }
        );
        let period = if use_end {
            format!("[~{}]", format_jst_short(date))
        } else {
            format!("[{}~]", format_jst_short(date))
        };
        let title = format!(
            "{}{period}{}",
            if active { "🟢 " } else { "" },
            if permanent { " 常設" } else { "" }
        );
        let label = row.label.replace('`', " ").replace('\r', "");
        let label = if name == "mission" {
            label
        } else {
            label.replace('\n', " ")
        };
        let indented = label
            .split('\n')
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let indented = truncate_utf16(&indented, 244);
        if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
            group.labels.push(indented);
        } else {
            groups.push(CodeGroup {
                key,
                title,
                labels: vec![indented],
            });
        }
    }
    if name == "mission" && visible.len() > shown {
        groups.push(CodeGroup {
            key: "omitted".to_owned(),
            title: String::new(),
            labels: mission_footers(visible[shown..].iter().filter_map(|row| row.id).collect()),
        });
    }
    format_code_blocks(name, groups, "\n")
}

fn format_changes(data: &AddedScheduleData, now: DateTime<Utc>) -> Result<Vec<String>, String> {
    let mut rows: Vec<(&str, String, &ScheduleHeader, &ScheduleHeader)> = Vec::new();
    if let Some(gatya) = &data.gatya {
        for change in &data.changes.gatya {
            let Some(entry) = change.after.gachas.first() else {
                continue;
            };
            if entry.id >= 0 {
                rows.push((
                    "gatya",
                    gacha_label(entry, change.after.header.gacha_type, gatya),
                    &change.before.header.schedule,
                    &change.after.header.schedule,
                ));
            }
        }
    }
    if let Some(sale) = &data.sale {
        for change in &data.changes.sale {
            let Some(id) = change.after.stage_ids.first().copied() else {
                continue;
            };
            rows.push((
                if is_mission_id(id) { "mission" } else { "sale" },
                format!("{id} {}", stage_name_preserving_breaks(id, sale)),
                &change.before.header,
                &change.after.header,
            ));
        }
    }
    if let Some(item) = &data.item {
        for change in &data.changes.item {
            rows.push((
                "item",
                item_label(&change.after, item),
                &change.before.header,
                &change.after.header,
            ));
        }
    }
    let mut seen = HashSet::new();
    rows.retain(|(kind, label, before, after)| {
        seen.insert(format!(
            "{kind}\0{label}\0{}\0{}",
            header_key(before),
            header_key(after)
        )) && is_visible(after, now).unwrap_or(false)
    });
    let order = ["gatya", "sale", "item", "mission"];
    rows.sort_by_key(|(kind, _, _, _)| {
        order
            .iter()
            .position(|value| value == kind)
            .unwrap_or(order.len())
    });
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let labels = rows
        .into_iter()
        .map(|(kind, label, before, after)| {
            let mut details = Vec::new();
            if before.start_date != after.start_date || before.start_time != after.start_time {
                details.push(format!(
                    "開始: {} → {}",
                    date_time(&before.start_date, &before.start_time),
                    date_time(&after.start_date, &after.start_time)
                ));
            }
            if before.end_date != after.end_date || before.end_time != after.end_time {
                details.push(format!(
                    "終了: {} → {}",
                    end_date_time(before),
                    end_date_time(after)
                ));
            }
            if before.min_version != after.min_version {
                details.push(format!(
                    "必要Ver: {} → {}",
                    before.min_version, after.min_version
                ));
            }
            if before.max_version != after.max_version {
                details.push(format!(
                    "上限Ver: {} → {}",
                    before.max_version, after.max_version
                ));
            }
            truncate_utf16(
                &format!(
                    "[{kind}] {}\n  {}",
                    truncate_utf16(&label, 120),
                    details.join("\n  ")
                )
                .replace('`', " ")
                .replace('\r', ""),
                360,
            )
        })
        .collect();
    format_code_blocks(
        "変更",
        vec![CodeGroup {
            key: String::new(),
            title: String::new(),
            labels,
        }],
        "\n\n",
    )
}

fn format_code_blocks(
    name: &str,
    groups: Vec<CodeGroup>,
    separator: &str,
) -> Result<Vec<String>, String> {
    let wrap = |lines: &[String]| format!("**{name}**\n```text\n{}\n```", lines.join(separator));
    let mut parts = Vec::new();
    let mut lines = Vec::new();
    let mut last_title = String::new();
    for group in groups {
        for label in group.labels {
            let title = if lines.is_empty() || last_title != group.title {
                group.title.clone()
            } else {
                String::new()
            };
            let mut next = lines.clone();
            if !title.is_empty() {
                next.push(title);
            }
            next.push(label.clone());
            if utf16_length(&wrap(&next)) > MESSAGE_LIMIT && !lines.is_empty() {
                parts.push(wrap(&lines));
                next = Vec::new();
                if !group.title.is_empty() {
                    next.push(group.title.clone());
                }
                next.push(label);
            }
            if utf16_length(&wrap(&next)) > MESSAGE_LIMIT {
                return Err(format!("schedule {name} entry exceeds message limit"));
            }
            lines = next;
            last_title = group.title.clone();
        }
    }
    if !lines.is_empty() {
        parts.push(wrap(&lines));
    }
    Ok(parts)
}

fn mission_footers(ids: Vec<i64>) -> Vec<String> {
    let prefix = format!("その他{}件(", ids.len());
    let mut output = Vec::new();
    let mut values: Vec<String> = Vec::new();
    for id in ids {
        let next = values
            .iter()
            .cloned()
            .chain(std::iter::once(id.to_string()))
            .collect::<Vec<_>>();
        if utf16_length(&format!("{prefix}{})", next.join(","))) > 1_900 && !values.is_empty() {
            output.push(format!("{prefix}{})", values.join(",")));
            values.clear();
        }
        values.push(id.to_string());
    }
    if !values.is_empty() {
        output.push(format!("{prefix}{})", values.join(",")));
    }
    output
}

fn row_key(row: &Row) -> String {
    format!("{}\0{}", header_key(&row.header), row.label)
}

fn header_key(header: &ScheduleHeader) -> String {
    format!(
        "{}\0{}\0{}\0{}\0{}\0{}",
        header.start_date,
        header.start_time,
        header.end_date,
        header.end_time,
        header.min_version,
        header.max_version
    )
}

fn is_permanent(header: &ScheduleHeader) -> bool {
    header.end_date == "20300101"
}

fn is_visible(header: &ScheduleHeader, now: DateTime<Utc>) -> Result<bool, String> {
    Ok(is_permanent(header) || parse_header_date(&header.end_date, &header.end_time)? > now)
}

fn date_time(date: &str, time: &str) -> String {
    let date = format!("{date:0>8}");
    let time = format!("{time:0>4}");
    format!(
        "{}/{}/{} {}:{}",
        date.get(0..4).unwrap_or("0000"),
        date.get(4..6).unwrap_or("00"),
        date.get(6..8).unwrap_or("00"),
        time.get(0..2).unwrap_or("00"),
        time.get(2..4).unwrap_or("00")
    )
}

fn end_date_time(header: &ScheduleHeader) -> String {
    if is_permanent(header) {
        "常設".to_owned()
    } else {
        date_time(&header.end_date, &header.end_time)
    }
}

fn truncate_utf16(value: &str, maximum: usize) -> String {
    let mut output = String::new();
    let mut length = 0;
    for character in value.chars() {
        let next = length + character.len_utf16();
        if next > maximum {
            break;
        }
        output.push(character);
        length = next;
    }
    output
}

fn utf16_length(value: &str) -> usize {
    value.encode_utf16().count()
}

pub(super) fn history_url(paths: &[String]) -> String {
    let timestamp = paths
        .iter()
        .filter_map(|path| {
            path.strip_suffix(".tsv")?
                .rsplit_once('_')?
                .1
                .parse::<i64>()
                .ok()
        })
        .max();
    let mut url = reqwest::Url::parse("https://kbc-rakv0-event.vercel.app/")
        .expect("fixed event site URL is valid");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("tab", "history");
        if let Some(timestamp) = timestamp {
            query.append_pair("tsv", &timestamp.to_string());
        }
        query.append_pair("type", "all");
    }
    url.to_string()
}
