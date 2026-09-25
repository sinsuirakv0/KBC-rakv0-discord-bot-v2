//! 保存済みSchedule更新を表示する`skd` Command。

pub(crate) mod data_source;
mod diff;
mod formatter;
mod parser;

use std::sync::Arc;

use chrono::{Datelike, FixedOffset, NaiveDate, Timelike};
use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::services::HttpService;
use crate::storage::StorageService;

use data_source::{ScheduleType, SkdDataSource, SkdUpdate};

const USAGE: &str = "使い方: o.skd / o.skd 2026/07/30 / o.skd 2026 07 30";
const DATA_ERROR_MESSAGE: &str = "スケジュール更新の取得に失敗しました。";
const EMPTY_MESSAGE: &str = "保存済みのスケジュール更新はありません。";

pub(super) struct SkdCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: SkdDataSource,
    storage: Arc<StorageService>,
}

impl SkdCommand {
    pub(super) fn new(help: &str, http: Arc<HttpService>, storage: Arc<StorageService>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "skd".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            data_source: SkdDataSource::new(http),
            storage,
        }
    }

    async fn run(
        &self,
        context: &CommandContext,
        arguments: &[String],
    ) -> Result<CommandOutput, crate::command::CommandExecutionError> {
        if arguments.len() == 1 && arguments[0].eq_ignore_ascii_case("help") {
            return Ok(message(context.channel_id(), &self.help));
        }
        let date = match parse_date(arguments) {
            Ok(date) => date,
            Err(()) => return Ok(message(context.channel_id(), USAGE)),
        };
        let update = match self.data_source.load(date).await {
            Ok(Some(update)) => update,
            Ok(None) => return Ok(message(context.channel_id(), EMPTY_MESSAGE)),
            Err(error) => {
                eprintln!("Schedule update retrieval failed: {error}");
                return Ok(message(context.channel_id(), DATA_ERROR_MESSAGE));
            }
        };
        let related_urls = match self
            .storage
            .skd_related_urls(context.guild_id().expect("guild-only command has a guild"))
            .await
        {
            Ok(urls) => urls,
            Err(error) => {
                eprintln!("SKD related site settings load failed: {error}");
                Vec::new()
            }
        };
        format_update(context.channel_id(), update, &related_urls)
    }
}

impl Command for SkdCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(&context, &arguments).await })
    }
}

fn parse_date(arguments: &[String]) -> Result<Option<NaiveDate>, ()> {
    if arguments.is_empty() {
        return Ok(None);
    }
    let joined = arguments.join(" ");
    let parts = joined
        .split(|character: char| character == '/' || character.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0].len() != 4
        || !parts
            .iter()
            .all(|part| part.chars().all(|character| character.is_ascii_digit()))
    {
        return Err(());
    }
    let year = parts[0].parse().map_err(|_| ())?;
    let month = parts[1].parse().map_err(|_| ())?;
    let day = parts[2].parse().map_err(|_| ())?;
    if year < 1_000 {
        return Err(());
    }
    NaiveDate::from_ymd_opt(year, month, day)
        .map(Some)
        .ok_or(())
}

fn format_update(
    channel_id: &str,
    update: SkdUpdate,
    related_urls: &[String],
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let timezone = FixedOffset::east_opt(9 * 60 * 60).expect("JST offset is valid");
    let date = update.timestamp.with_timezone(&timezone);
    let weekday =
        ["日", "月", "火", "水", "木", "金", "土"][date.weekday().num_days_from_sunday() as usize];
    let types = format_types(&update.types);
    let note = if update.initial_types.is_empty() {
        String::new()
    } else {
        format!(
            "\n初回保存分（比較元なし）: {}",
            format_types(&update.initial_types)
        )
    };
    let header = format!(
        "**スケジュール更新**\n検知時刻（TSV保存時刻）: {:04}/{:02}/{:02}({weekday}) {:02}:{:02}:{:02}\n種類: {types}{note}",
        date.year(),
        date.month(),
        date.day(),
        date.hour(),
        date.minute(),
        date.second(),
    );
    let mut output = CommandOutput::new();
    output.push(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: header,
    })?;
    for content in update.contents {
        let content = with_related_sites(&content, related_urls).unwrap_or(content);
        output.push(CoreActionData::SendMessage {
            channel_id: channel_id.to_owned(),
            content,
        })?;
    }
    Ok(output)
}

pub(crate) fn with_related_sites(content: &str, urls: &[String]) -> Option<String> {
    let body = content
        .strip_prefix("**関連サイト**\n")
        .or_else(|| content.strip_prefix("**KBC**\n"));
    let Some(body) = body else {
        return Some(content.to_owned());
    };
    let mut output = format!("**関連サイト**\n{body}");
    for url in urls {
        output.push_str(&format!("\n<{url}>"));
    }
    (output.encode_utf16().count() <= 2_000).then_some(output)
}

fn format_types(types: &[ScheduleType]) -> String {
    types
        .iter()
        .map(|data_type| data_type.label())
        .collect::<Vec<_>>()
        .join(",")
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::with_related_sites;

    #[test]
    fn appends_guild_related_sites() {
        let content = with_related_sites(
            "**KBC**\n<https://kbc.example/history>",
            &["https://example.com/".to_owned()],
        );

        assert_eq!(
            content.as_deref(),
            Some("**関連サイト**\n<https://kbc.example/history>\n<https://example.com/>")
        );
    }
}
