//! 外部Event dataを使うitem Command。

mod data_source;
pub(in crate::commands) mod model;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::services::Clock;

use super::common::output::{push_chunked, push_message};
use super::common::schedule::{
    format_jst_full, format_jst_short, format_time_block, parse_header_date,
};
use data_source::ItemDataSource;
use model::{ItemDisplayData, ItemEntry, ItemJson};

pub(super) use data_source::ItemDataSource as RegisteredItemDataSource;

const USAGE: &str = "❌ 使い方:\n　`o.item` — アイテム配布一覧\n　`o.item <ID>` — giftType/eventIDで検索\n　`o.item <ID> j` — JSON表示\n　`o.item <ID> r` — Raw表示";
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

pub(super) struct ItemCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: ItemDataSource,
    clock: Arc<dyn Clock>,
}

impl ItemCommand {
    pub(super) fn new(help: &str, data_source: ItemDataSource, clock: Arc<dyn Clock>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "item".to_owned(),
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
            return Ok(CommandOutput::single(CoreActionData::SendMessage {
                channel_id: channel_id.to_owned(),
                content: self.help.clone(),
            }));
        }

        let request = parse_request(arguments);
        match request {
            ItemRequest::Usage => single_message(channel_id, USAGE),
            ItemRequest::Schedule => match self.data_source.fetch_display_data().await {
                Ok(data) => {
                    let content = format_schedule(&data, self.clock.now())
                        .unwrap_or_else(|| "開催中・予定のアイテム配布はありません".to_owned());
                    chunked_message(channel_id, &content, "")
                }
                Err(error) => data_error(channel_id, error),
            },
            ItemRequest::Detail { id } => match self.data_source.fetch_display_data().await {
                Ok(data) => format_details(channel_id, id, &data),
                Err(error) => data_error(channel_id, error),
            },
            ItemRequest::Json { id } | ItemRequest::Raw { id } => {
                let is_raw = matches!(request, ItemRequest::Raw { .. });
                match self.data_source.fetch_item_json().await {
                    Ok(item) => format_json_or_raw(channel_id, id, is_raw, &item),
                    Err(error) => data_error(channel_id, error),
                }
            }
        }
    }
}

impl Command for ItemCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(context.channel_id(), &arguments).await })
    }
}

enum ItemRequest {
    Schedule,
    Detail { id: i64 },
    Json { id: i64 },
    Raw { id: i64 },
    Usage,
}

fn parse_request(arguments: &[String]) -> ItemRequest {
    if arguments.is_empty() {
        return ItemRequest::Schedule;
    }
    let id = arguments.first().and_then(|value| parse_id(value));
    if arguments.len() == 1 {
        return id.map_or(ItemRequest::Usage, |id| ItemRequest::Detail { id });
    }
    if arguments.len() == 2 {
        let Some(id) = id else {
            return ItemRequest::Usage;
        };
        if arguments[1].eq_ignore_ascii_case("j") || arguments[1].eq_ignore_ascii_case("json") {
            return ItemRequest::Json { id };
        }
        if arguments[1].eq_ignore_ascii_case("r") || arguments[1].eq_ignore_ascii_case("raw") {
            return ItemRequest::Raw { id };
        }
    }
    ItemRequest::Usage
}

fn parse_id(value: &str) -> Option<i64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id = value.parse().ok()?;
    (id <= MAX_SAFE_INTEGER).then_some(id)
}

fn format_details(
    channel_id: &str,
    id: i64,
    data: &ItemDisplayData,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let (entries, searched_by_event_id) = search_entries(id, &data.item);
    if entries.is_empty() {
        return single_message(channel_id, not_found_message(id));
    }

    let mut output = CommandOutput::new();
    if searched_by_event_id {
        push_message(
            &mut output,
            channel_id,
            format!("ℹ️ giftType `{id}` では見つからなかった為、eventID で検索しました"),
        )?;
    }
    for entry in entries {
        push_chunked(&mut output, channel_id, &format_detail(entry, data), "")?;
    }
    Ok(output)
}

fn format_json_or_raw(
    channel_id: &str,
    id: i64,
    is_raw: bool,
    item: &ItemJson,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let (entries, searched_by_event_id) = search_entries(id, item);
    if entries.is_empty() {
        return single_message(channel_id, not_found_message(id));
    }

    let mut output = CommandOutput::new();
    if searched_by_event_id {
        push_message(
            &mut output,
            channel_id,
            format!("ℹ️ giftType `{id}` では見つからなかった為、eventID で検索しました"),
        )?;
    }
    for entry in entries {
        if is_raw {
            match &entry.raw {
                Some(raw) => push_chunked(&mut output, channel_id, &raw.replace('\t', "    "), "")?,
                None => push_message(
                    &mut output,
                    channel_id,
                    format!(
                        "❌ (startDate: `{}`) に raw データがありません",
                        entry.header.start_date
                    ),
                )?,
            }
        } else {
            match serde_json::to_string_pretty(entry) {
                Ok(json) => push_chunked(&mut output, channel_id, &json, "json")?,
                Err(error) => return data_error(channel_id, error),
            }
        }
    }
    Ok(output)
}

fn search_entries(id: i64, item: &ItemJson) -> (Vec<&ItemEntry>, bool) {
    let by_gift_type = item
        .data
        .iter()
        .filter(|entry| entry.gift.gift_type == id)
        .collect::<Vec<_>>();
    if !by_gift_type.is_empty() {
        return (by_gift_type, false);
    }
    (
        item.data
            .iter()
            .filter(|entry| entry.gift.event_id == id)
            .collect(),
        true,
    )
}

fn format_detail(entry: &ItemEntry, data: &ItemDisplayData) -> String {
    let start = header_start(entry);
    let end_text = if is_permanent(entry) {
        "常設".to_owned()
    } else {
        format_jst_full(header_end(entry))
    };
    let amount = if entry.gift.gift_amount > 0 {
        format!(" ×{}", entry.gift.gift_amount)
    } else {
        String::new()
    };
    let mut lines = vec![
        data.item_names
            .get(&entry.gift.gift_type)
            .map(|item| item.name.as_str())
            .unwrap_or("不明")
            .to_owned(),
        format!(
            "{} ~ {}  ver.{}~{}",
            format_jst_full(start),
            end_text,
            entry.header.min_version,
            entry.header.max_version
        ),
        format!("eventId: {}", entry.gift.event_id),
        format!("giftType: {}{amount}", entry.gift.gift_type),
    ];
    if entry.gift.repeat_flag == 0 {
        lines.push("1回限り".to_owned());
    }

    let gift_detail = data
        .item_names
        .get(&entry.gift.gift_type)
        .map(|item| format_gift_detail(&item.detail))
        .unwrap_or_default();
    if !gift_detail.is_empty() {
        lines.extend([String::new(), "ギフト詳細".to_owned(), gift_detail]);
    }

    let mut extras = Vec::new();
    if !entry.gift.title.is_empty() {
        extras.push(entry.gift.title.clone());
    }
    if !entry.gift.message.is_empty() {
        extras.push(replace_html_breaks(&entry.gift.message));
    }
    if !entry.gift.url.is_empty() {
        extras.push(entry.gift.url.clone());
    }
    if !extras.is_empty() {
        lines.push(String::new());
        lines.extend(extras);
    }
    if !entry.time_blocks.is_empty() {
        lines.push(String::new());
        lines.extend(
            entry
                .time_blocks
                .iter()
                .map(|block| format!("・{}", format_time_block(block))),
        );
    }
    lines.join("\n")
}

fn format_schedule(data: &ItemDisplayData, now: DateTime<Utc>) -> Option<String> {
    let mut entries = data
        .item
        .data
        .iter()
        .filter(|entry| !is_permanent(entry) && header_end(entry) > now)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| header_start(entry));

    struct Group<'a> {
        key: String,
        active: bool,
        period: String,
        entries: Vec<&'a ItemEntry>,
    }
    let mut groups: Vec<Group<'_>> = Vec::new();
    for entry in entries {
        let start = header_start(entry);
        let end = header_end(entry);
        let active = now >= start && now < end;
        let date_key = if active {
            &entry.header.end_date
        } else {
            &entry.header.start_date
        };
        let key = format!("{}:{date_key}", if active { "active" } else { "upcoming" });
        if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
            group.entries.push(entry);
        } else {
            groups.push(Group {
                key,
                active,
                period: if active {
                    format!("~{}", format_jst_short(end))
                } else {
                    format!("{}~", format_jst_short(start))
                },
                entries: vec![entry],
            });
        }
    }

    let mut lines = Vec::new();
    for group in groups {
        lines.push(format!(
            "{}[{}]",
            if group.active { "🟢 " } else { "" },
            group.period
        ));
        for entry in group.entries {
            let amount = if entry.gift.gift_amount > 0 {
                format!(" ×{}", entry.gift.gift_amount)
            } else {
                String::new()
            };
            lines.push(format!(
                "    {} {}{amount}",
                entry.gift.gift_type,
                schedule_name(entry, data)
            ));
        }
    }
    (!lines.is_empty()).then(|| {
        format!(
            "アイテムスケジュール [開催中＆予定]\n\n{}",
            lines.join("\n")
        )
    })
}

pub(in crate::commands) fn schedule_name<'a>(
    entry: &'a ItemEntry,
    data: &'a ItemDisplayData,
) -> &'a str {
    if matches!(entry.gift.gift_type, 301 | 302) {
        return data
            .sale_names
            .get(&entry.gift.gift_type)
            .map(String::as_str)
            .or_else(|| {
                data.item_names
                    .get(&entry.gift.gift_type)
                    .map(|item| item.name.as_str())
            })
            .filter(|name| !name.is_empty())
            .or_else(|| (!entry.gift.title.trim().is_empty()).then_some(entry.gift.title.trim()))
            .unwrap_or("不明");
    }
    if !entry.gift.title.trim().is_empty() {
        entry.gift.title.trim()
    } else {
        data.item_names
            .get(&entry.gift.gift_type)
            .map(|item| item.name.as_str())
            .unwrap_or("不明")
    }
}

fn format_gift_detail(value: &str) -> String {
    let decoded = decode_html_entities(value);
    let with_breaks = replace_html_breaks(&decoded);
    let mut result = String::new();
    let mut inside_tag = false;
    for character in with_breaks.chars() {
        match character {
            '<' => inside_tag = true,
            '>' if inside_tag => inside_tag = false,
            _ if !inside_tag => result.push(character),
            _ => {}
        }
    }
    let normalized_newlines = result.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized_newlines
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>();
    let mut normalized = lines.join("\n");
    while normalized.contains("\n\n\n") {
        normalized = normalized.replace("\n\n\n", "\n\n");
    }
    normalized.trim().to_owned()
}

fn replace_html_breaks(value: &str) -> String {
    let mut result = value.to_owned();
    for pattern in ["<br>", "<BR>", "<br/>", "<BR/>", "<br />", "<BR />"] {
        result = result.replace(pattern, "\n");
    }
    result
}

fn decode_html_entities(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remainder = value;
    while let Some(start) = remainder.find('&') {
        result.push_str(&remainder[..start]);
        let entity_start = &remainder[start..];
        let Some(end) = entity_start.find(';') else {
            result.push_str(entity_start);
            return result;
        };
        let code = &entity_start[1..end];
        let decoded = match code.to_ascii_lowercase().as_str() {
            "amp" => Some('&'),
            "apos" => Some('\''),
            "gt" => Some('>'),
            "lt" => Some('<'),
            "nbsp" => Some(' '),
            "quot" => Some('"'),
            _ if code.starts_with("#x") || code.starts_with("#X") => {
                u32::from_str_radix(&code[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if code.starts_with('#') => code[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(character) = decoded {
            result.push(character);
        } else {
            result.push_str(&entity_start[..=end]);
        }
        remainder = &entity_start[end + 1..];
    }
    result.push_str(remainder);
    result
}

fn header_start(entry: &ItemEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.start_date, &entry.header.start_time)
        .expect("item headers are validated when loaded")
}

fn header_end(entry: &ItemEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.end_date, &entry.header.end_time)
        .expect("item headers are validated when loaded")
}

fn is_permanent(entry: &ItemEntry) -> bool {
    entry.header.end_date == "20300101"
}

fn not_found_message(id: i64) -> String {
    format!("❌ `{id}` は giftType・eventID のどちらでも見つかりませんでした")
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
    eprintln!("Item data retrieval failed: {error}");
    single_message(channel_id, "❌ データ取得に失敗しました")
}

#[cfg(test)]
mod tests {
    use super::{ItemRequest, parse_request};

    #[test]
    fn parses_supported_item_requests() {
        assert!(matches!(parse_request(&[]), ItemRequest::Schedule));
        assert!(matches!(
            parse_request(&["828".to_owned(), "JSON".to_owned()]),
            ItemRequest::Json { id: 828 }
        ));
        assert!(matches!(
            parse_request(&["828".to_owned(), "other".to_owned()]),
            ItemRequest::Usage
        ));
    }
}
