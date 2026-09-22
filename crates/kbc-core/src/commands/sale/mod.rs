//! 外部Event dataを使うsale Command。

mod data_source;
pub(in crate::commands) mod model;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::services::Clock;
use crate::session::{
    SessionContext, SessionContinuation, SessionFuture, SessionRequest, SessionResume,
};

use super::common::output::{push_chunked, push_message};
use super::common::schedule::{
    format_duration, format_jst_full, format_jst_short, format_time_block, parse_header_date,
};
use data_source::SaleDataSource;
use model::{SaleDisplayData, SaleEntry, SaleJson};

pub(super) use data_source::SaleDataSource as RegisteredSaleDataSource;

const NUMBER_EMOJIS: [&str; 9] = ["1️⃣", "2️⃣", "3️⃣", "4️⃣", "5️⃣", "6️⃣", "7️⃣", "8️⃣", "9️⃣"];
const SELECTION_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct SaleCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: SaleDataSource,
    clock: Arc<dyn Clock>,
}

impl SaleCommand {
    pub(super) fn new(help: &str, data_source: SaleDataSource, clock: Arc<dyn Clock>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "sale".to_owned(),
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
        context: &CommandContext,
        arguments: &[String],
    ) -> Result<CommandOutput, crate::command::CommandExecutionError> {
        let channel_id = context.channel_id();
        if arguments
            .first()
            .is_some_and(|argument| argument.eq_ignore_ascii_case("help"))
        {
            return single_message(channel_id, self.help.clone());
        }

        match parse_request(arguments) {
            SaleRequest::Schedule => match self.data_source.fetch_display_data().await {
                Ok(data) => {
                    let content = format_schedule(&data, self.clock.now())
                        .unwrap_or_else(|| "開催中・予定のセールイベントはありません".to_owned());
                    chunked_message(channel_id, &content, "")
                }
                Err(error) => data_error(channel_id, error),
            },
            SaleRequest::Detail { id } => match self.data_source.fetch_display_data().await {
                Ok(data) => format_details(channel_id, id, &data),
                Err(error) => data_error(channel_id, error),
            },
            SaleRequest::Json { id } => match self.data_source.fetch_sale_json().await {
                Ok(sale) => format_json_or_raw(channel_id, id, false, &sale),
                Err(error) => data_error(channel_id, error),
            },
            SaleRequest::Raw { id } => match self.data_source.fetch_sale_json().await {
                Ok(sale) => format_json_or_raw(channel_id, id, true, &sale),
                Err(error) => data_error(channel_id, error),
            },
            SaleRequest::Search { query } => match self.data_source.fetch_display_data().await {
                Ok(data) => format_search(context, &query, data),
                Err(error) => data_error(channel_id, error),
            },
        }
    }
}

impl Command for SaleCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(&context, &arguments).await })
    }
}

enum SaleRequest {
    Schedule,
    Detail { id: i64 },
    Json { id: i64 },
    Raw { id: i64 },
    Search { query: String },
}

fn parse_request(arguments: &[String]) -> SaleRequest {
    if arguments.is_empty() {
        return SaleRequest::Schedule;
    }
    if let Ok(id) = arguments[0].trim().parse() {
        return match arguments.get(1).map(|value| value.to_ascii_lowercase()) {
            Some(modifier) if modifier == "j" || modifier == "json" => SaleRequest::Json { id },
            Some(modifier) if modifier == "r" || modifier == "raw" => SaleRequest::Raw { id },
            _ => SaleRequest::Detail { id },
        };
    }
    SaleRequest::Search {
        query: arguments.join(" ").trim().to_owned(),
    }
}

fn format_details(
    channel_id: &str,
    id: i64,
    data: &SaleDisplayData,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let entries = data
        .sale
        .data
        .iter()
        .filter(|entry| entry.stage_ids.contains(&id))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return single_message(
            channel_id,
            format!("❌ ID `{id}` は sale.json に含まれていません"),
        );
    }
    let mut output = CommandOutput::new();
    for entry in entries {
        push_chunked(&mut output, channel_id, &format_detail(entry, data, id), "")?;
    }
    Ok(output)
}

fn format_json_or_raw(
    channel_id: &str,
    id: i64,
    is_raw: bool,
    sale: &SaleJson,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let entries = sale
        .data
        .iter()
        .filter(|entry| entry.stage_ids.contains(&id))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return single_message(
            channel_id,
            format!("❌ ID `{id}` は sale.json に含まれていません"),
        );
    }
    let mut output = CommandOutput::new();
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

fn format_search(
    context: &CommandContext,
    query: &str,
    data: SaleDisplayData,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let channel_id = context.channel_id();
    let normalized = query.to_lowercase();
    let matches = data
        .merged_search_names()
        .into_iter()
        .filter(|(id, name)| !is_mission_id(*id) && name.to_lowercase().contains(&normalized))
        .map(|(id, name)| (id, strip_display_markup(name, " ")))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return single_message(
            channel_id,
            format!("❌ `{query}` に一致するイベントは見つかりませんでした"),
        );
    }

    if matches.len() <= NUMBER_EMOJIS.len() {
        let lines = matches
            .iter()
            .enumerate()
            .map(|(index, (id, name))| format!("{} {id} {name}", NUMBER_EMOJIS[index]))
            .collect::<Vec<_>>();
        let reactions = NUMBER_EMOJIS[..matches.len()]
            .iter()
            .map(|emoji| (*emoji).to_owned())
            .collect();
        let stage_ids = matches.iter().map(|(id, _)| *id).collect();
        let session = SessionRequest::new(
            context.user_id().to_owned(),
            channel_id.to_owned(),
            SELECTION_TIMEOUT,
            Box::new(SaleSelection {
                channel_id: channel_id.to_owned(),
                reactions,
                stage_ids,
                data,
            }),
        )?
        .clear_reactions_on_completion();
        return Ok(CommandOutput::with_session(
            CoreActionData::SendMessage {
                channel_id: channel_id.to_owned(),
                content: format!("```\n{}\n```", lines.join("\n")),
            },
            session,
        ));
    }

    let lines = matches
        .iter()
        .map(|(id, name)| format!("{id} {name}"))
        .collect::<Vec<_>>();
    chunked_message(
        channel_id,
        &format!(
            "「{query}」の検索結果 ({}件)\n\n{}",
            matches.len(),
            lines.join("\n")
        ),
        "",
    )
}

struct SaleSelection {
    channel_id: String,
    reactions: Vec<String>,
    stage_ids: Vec<i64>,
    data: SaleDisplayData,
}

impl SessionContinuation for SaleSelection {
    fn reactions(&self) -> &[String] {
        &self.reactions
    }

    fn resume(self: Box<Self>, _context: SessionContext, reaction: String) -> SessionFuture {
        Box::pin(async move {
            let Some(index) = self
                .reactions
                .iter()
                .position(|candidate| candidate == &reaction)
            else {
                return Ok(SessionResume::complete(Vec::new()));
            };
            format_details(&self.channel_id, self.stage_ids[index], &self.data)
                .map(|output| SessionResume::complete(output.into_actions().collect()))
        })
    }
}

fn format_detail(entry: &SaleEntry, data: &SaleDisplayData, selected_id: i64) -> String {
    let mut lines = Vec::new();
    let representative_id = representative_stage_id(entry, &data.card_setting_stage_ids);
    if representative_id == Some(selected_id) {
        lines.push(format!("{selected_id} {}", stage_name(selected_id, data)));
        let targets = entry
            .stage_ids
            .iter()
            .filter(|id| **id != selected_id)
            .map(|id| format!("{id} {}", stage_name(*id, data)))
            .collect::<Vec<_>>();
        if !targets.is_empty() {
            lines.push(format!("対象ステージ: {}", targets.join("、")));
        }
    } else {
        lines.push(format!("{selected_id} {}", stage_name(selected_id, data)));
    }

    let start = header_start(entry);
    let end_text = if is_permanent(entry) {
        "常設".to_owned()
    } else {
        format_jst_full(header_end(entry))
    };
    lines.push(format!(
        "{} ~ {}  ver.{}~{}",
        format_jst_full(start),
        end_text,
        entry.header.min_version,
        entry.header.max_version
    ));
    if entry.time_blocks.is_empty() {
        lines.push("・常時開催（時間制限なし）".to_owned());
    } else {
        lines.extend(
            entry
                .time_blocks
                .iter()
                .map(|block| format!("・{}", format_time_block(block))),
        );
    }
    lines.join("\n")
}

fn format_schedule(data: &SaleDisplayData, now: DateTime<Utc>) -> Option<String> {
    let mut entries = data
        .sale
        .data
        .iter()
        .filter(|entry| !is_permanent(entry) && header_end(entry) > now)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| header_start(entry));

    struct ScheduleItem {
        active: bool,
        period: String,
        start_date_key: String,
        end_date_key: String,
        stage_ids: Vec<i64>,
        duration: String,
    }
    struct Group {
        key: String,
        active: bool,
        period: String,
        items: Vec<ScheduleItem>,
    }

    let mut groups: Vec<Group> = Vec::new();
    for entry in entries {
        let stage_ids = list_stage_ids(entry, &data.card_setting_stage_ids);
        if stage_ids.is_empty() {
            continue;
        }
        let start = header_start(entry);
        let end = header_end(entry);
        let active = now >= start && now < end;
        let item = ScheduleItem {
            active,
            period: if active {
                format!("~{}", format_jst_short(end))
            } else {
                format!("{}~", format_jst_short(start))
            },
            start_date_key: normalized_date_key(&entry.header.start_date),
            end_date_key: normalized_date_key(&entry.header.end_date),
            stage_ids,
            duration: format_duration(start, end),
        };
        let date_key = if active {
            &item.end_date_key
        } else {
            &item.start_date_key
        };
        let key = format!("{}:{date_key}", if active { "active" } else { "upcoming" });
        if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
            group.items.push(item);
        } else {
            groups.push(Group {
                key,
                active: item.active,
                period: item.period.clone(),
                items: vec![item],
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
        for item in group.items {
            for id in item.stage_ids {
                lines.push(format!(
                    "    {id} {} {}",
                    stage_name(id, data),
                    item.duration
                ));
            }
        }
    }
    (!lines.is_empty())
        .then(|| format!("セールスケジュール [開催中＆予定]\n\n{}", lines.join("\n")))
}

pub(in crate::commands) fn stage_name(id: i64, data: &SaleDisplayData) -> String {
    stage_name_with_breaks(id, data, " ")
}

pub(in crate::commands) fn stage_name_preserving_breaks(id: i64, data: &SaleDisplayData) -> String {
    stage_name_with_breaks(id, data, "\n")
}

fn stage_name_with_breaks(id: i64, data: &SaleDisplayData, line_break: &str) -> String {
    if is_mission_id(id) {
        let lookup_id = if (15_000..=15_999).contains(&id) {
            id - 15_000
        } else {
            id
        };
        let Some(raw_name) = data.mission_names.get(lookup_id) else {
            return "不明".to_owned();
        };
        let name = raw_name.split([',', '，']).next().unwrap_or(raw_name);
        return strip_display_markup(name, line_break);
    }
    data.sale_names
        .get(id)
        .or_else(|| data.all_day_event_names.get(id))
        .map(|name| strip_display_markup(name, line_break))
        .unwrap_or_else(|| "不明".to_owned())
}

fn strip_display_markup(value: &str, line_break: &str) -> String {
    let with_breaks = replace_breaks(value, line_break);
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
    result.trim().to_owned()
}

fn replace_breaks(value: &str, replacement: &str) -> String {
    let mut result = value.to_owned();
    for pattern in ["<br>", "<BR>", "<br/>", "<BR/>", "<br />", "<BR />"] {
        result = result.replace(pattern, replacement);
    }
    result
}

pub(in crate::commands) fn is_mission_id(id: i64) -> bool {
    (8_000..=9_999).contains(&id)
        || (15_000..=15_999).contains(&id)
        || (17_000..=17_999).contains(&id)
}

fn representative_stage_id(entry: &SaleEntry, card_setting_ids: &[i64]) -> Option<i64> {
    if entry.stage_ids.len() <= 1 {
        return None;
    }
    card_setting_ids
        .iter()
        .copied()
        .find(|id| entry.stage_ids.contains(id))
}

pub(in crate::commands) fn list_stage_ids(entry: &SaleEntry, card_setting_ids: &[i64]) -> Vec<i64> {
    representative_stage_id(entry, card_setting_ids)
        .map(|id| vec![id])
        .unwrap_or_else(|| entry.stage_ids.clone())
        .into_iter()
        .filter(|id| !is_mission_id(*id))
        .collect()
}

fn normalized_date_key(value: &str) -> String {
    format!("{:0>8}", value.trim())
}

fn header_start(entry: &SaleEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.start_date, &entry.header.start_time)
        .expect("sale headers are validated when loaded")
}

fn header_end(entry: &SaleEntry) -> DateTime<Utc> {
    parse_header_date(&entry.header.end_date, &entry.header.end_time)
        .expect("sale headers are validated when loaded")
}

fn is_permanent(entry: &SaleEntry) -> bool {
    entry.header.end_date == "20300101"
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
    eprintln!("Sale data retrieval failed: {error}");
    single_message(channel_id, "❌ データ取得に失敗しました")
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::super::common::schedule::format_duration;
    use super::{SaleRequest, parse_request};

    #[test]
    fn parses_supported_sale_requests() {
        assert!(matches!(parse_request(&[]), SaleRequest::Schedule));
        assert!(matches!(
            parse_request(&["102".to_owned(), "JSON".to_owned()]),
            SaleRequest::Json { id: 102 }
        ));
        assert!(matches!(
            parse_request(&["補完".to_owned(), "ステージ".to_owned()]),
            SaleRequest::Search { .. }
        ));
    }

    #[test]
    fn formats_whole_duration_units() {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 3, 1, 2, 59).unwrap();
        assert_eq!(format_duration(start, end), "<2d1h2m>");
    }
}
