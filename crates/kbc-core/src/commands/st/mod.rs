//! ステージ名・マップ名・IDを検索する`st` Command。

mod data_source;

use std::sync::Arc;
use std::time::Duration;

use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::commands::common::file_picker::{NEXT_PAGE_EMOJI, NUMBER_EMOJIS, PREVIOUS_PAGE_EMOJI};
use crate::commands::common::search::normalize_search_text;
use crate::services::HttpService;
use crate::session::{
    SessionContext, SessionContinuation, SessionFuture, SessionRequest, SessionResume,
};

use data_source::{StageDataSource, StageEntry, StageSearchData};

const SEARCH_PAGE_URL: &str = "https://jarjarblink.github.io/JDB/map_search.html?cc=ja";
const DETAIL_PAGE_URL: &str = "https://jarjarblink.github.io/JDB/map.html";
const REACTION_TIMEOUT: Duration = Duration::from_secs(60);
const PAGE_SIZE: usize = 20;
const NOT_FOUND_MESSAGE: &str = "該当するステージが見つかりませんでした。";
const DATA_ERROR_MESSAGE: &str =
    "ステージデータを取得できませんでした。時間をおいて再度お試しください。";
const LIST_FOOTER: &str = "詳細は o.st <ID> で表示できます。";

pub(super) struct StCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: StageDataSource,
}

impl StCommand {
    pub(super) fn new(help: &str, http: Arc<HttpService>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "st".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            data_source: StageDataSource::new(http),
        }
    }

    async fn run(
        &self,
        context: &CommandContext,
        arguments: &[String],
    ) -> Result<CommandOutput, crate::command::CommandExecutionError> {
        let request = parse_request(arguments);
        let (query, force) = match request {
            StRequest::Landing => return Ok(message(context.channel_id(), SEARCH_PAGE_URL)),
            StRequest::Help => return Ok(message(context.channel_id(), &self.help)),
            StRequest::Search { query, force } => (query, force),
        };
        let data = match self.data_source.fetch().await {
            Ok(data) => data,
            Err(error) => {
                eprintln!("Stage data retrieval failed: {error}");
                return Ok(message(context.channel_id(), DATA_ERROR_MESSAGE));
            }
        };
        let matches = search_stages(&data, &query, force);
        if matches.is_empty() {
            return Ok(message(context.channel_id(), NOT_FOUND_MESSAGE));
        }
        if matches.len() <= 3 {
            let id_match = (!force).then(|| data.find_by_id(&query)).flatten();
            let mut output = CommandOutput::new();
            for entry in matches {
                output.push(CoreActionData::SendMessage {
                    channel_id: context.channel_id().to_owned(),
                    content: format_detail(
                        &entry,
                        id_match
                            .as_ref()
                            .is_some_and(|matched| Arc::ptr_eq(matched, &entry) && entry.is_map()),
                    ),
                })?;
            }
            return Ok(output);
        }
        if matches.len() <= NUMBER_EMOJIS.len() {
            let continuation = StageSelection::new(query, matches);
            let content = continuation.format("数字のリアクションで選択してください。");
            let session = SessionRequest::new(
                context.user_id().to_owned(),
                context.channel_id().to_owned(),
                REACTION_TIMEOUT,
                Box::new(continuation),
            )?
            .clear_reactions_on_completion();
            return Ok(CommandOutput::with_session(
                CoreActionData::SendMessage {
                    channel_id: context.channel_id().to_owned(),
                    content,
                },
                session,
            ));
        }
        list_output(context, query, matches)
    }
}

impl Command for StCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(&context, &arguments).await })
    }
}

enum StRequest {
    Landing,
    Help,
    Search { query: String, force: bool },
}

fn parse_request(arguments: &[String]) -> StRequest {
    if arguments.is_empty() {
        return StRequest::Landing;
    }
    if arguments.len() == 1 && arguments[0].eq_ignore_ascii_case("help") {
        return StRequest::Help;
    }
    let force =
        arguments[0].eq_ignore_ascii_case("-f") || arguments[0].eq_ignore_ascii_case("-force");
    let query = if force {
        arguments[1..].join(" ")
    } else {
        arguments.join(" ")
    };
    let query = query.trim().to_owned();
    if query.is_empty() {
        StRequest::Help
    } else {
        StRequest::Search { query, force }
    }
}

fn search_stages(data: &StageSearchData, query: &str, force: bool) -> Vec<Arc<StageEntry>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if !force && let Some(entry) = data.find_by_id(trimmed) {
        return vec![entry];
    }
    let searchable_query = if force {
        trimmed.to_owned()
    } else {
        normalize_search_text(trimmed)
    };
    let words = searchable_query.split_whitespace().collect::<Vec<_>>();
    data.maps
        .iter()
        .filter(|entry| {
            entry
                .search_names()
                .iter()
                .any(|name| name_matches(name, &words, force))
        })
        .chain(
            data.stages
                .iter()
                .filter(|entry| name_matches(&entry.display_name, &words, force)),
        )
        .cloned()
        .collect()
}

fn name_matches(name: &str, words: &[&str], force: bool) -> bool {
    let searchable = if force {
        name.to_owned()
    } else {
        normalize_search_text(name)
    };
    words.iter().all(|word| searchable.contains(word))
}

fn format_label(entry: &StageEntry) -> String {
    format!("{} {}", entry.display_id, entry.display_name)
}

fn format_url(entry: &StageEntry, raw_map_id: bool) -> String {
    if raw_map_id && entry.is_map() {
        return format!("{DETAIL_PAGE_URL}?cc=ja&id={}", entry.raw_map_id);
    }
    let mut url = format!(
        "{DETAIL_PAGE_URL}?cc=ja&type={}&map={}",
        entry.jdb_type, entry.jdb_map
    );
    if let Some(stage_index) = entry.stage_index() {
        url.push_str(&format!("&stage={stage_index}"));
    }
    url
}

fn format_detail(entry: &StageEntry, raw_map_id: bool) -> String {
    format!("{}\n{}", format_label(entry), format_url(entry, raw_map_id))
}

fn format_header(query: &str, total: usize, page: Option<(usize, usize)>) -> String {
    let page = page
        .map(|(index, count)| format!("・{}/{}ページ", index + 1, count))
        .unwrap_or_default();
    format!("ステージ「{query}」検索結果（{total}件{page}）")
}

fn format_selection(query: &str, matches: &[Arc<StageEntry>], footer: &str) -> String {
    let lines = matches
        .iter()
        .enumerate()
        .map(|(index, entry)| format!("{} {}", NUMBER_EMOJIS[index], format_label(entry)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\n```text\n{lines}\n```\n{footer}",
        format_header(query, matches.len(), None)
    )
}

fn format_page(
    query: &str,
    matches: &[Arc<StageEntry>],
    page_index: usize,
    footer: &str,
    include_page_count: bool,
) -> String {
    let page_count = matches.len().div_ceil(PAGE_SIZE);
    let lines = matches
        .iter()
        .skip(page_index * PAGE_SIZE)
        .take(PAGE_SIZE)
        .map(|entry| format_label(entry))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\n```text\n{lines}\n```\n{footer}",
        format_header(
            query,
            matches.len(),
            include_page_count.then_some((page_index, page_count))
        )
    )
}

fn list_output(
    context: &CommandContext,
    query: String,
    matches: Vec<Arc<StageEntry>>,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    if matches.len() <= PAGE_SIZE {
        return Ok(message(
            context.channel_id(),
            format_page(&query, &matches, 0, LIST_FOOTER, false),
        ));
    }
    let continuation = StagePages::new(query, Arc::new(matches), 0);
    let content = continuation.format(LIST_FOOTER);
    let session = SessionRequest::new(
        context.user_id().to_owned(),
        context.channel_id().to_owned(),
        REACTION_TIMEOUT,
        Box::new(continuation),
    )?
    .clear_reactions_on_completion();
    Ok(CommandOutput::with_session(
        CoreActionData::SendMessage {
            channel_id: context.channel_id().to_owned(),
            content,
        },
        session,
    ))
}

struct StageSelection {
    query: String,
    matches: Vec<Arc<StageEntry>>,
    reactions: Vec<String>,
}

impl StageSelection {
    fn new(query: String, matches: Vec<Arc<StageEntry>>) -> Self {
        let reactions = NUMBER_EMOJIS[..matches.len()]
            .iter()
            .map(|emoji| (*emoji).to_owned())
            .collect();
        Self {
            query,
            matches,
            reactions,
        }
    }

    fn format(&self, footer: &str) -> String {
        format_selection(&self.query, &self.matches, footer)
    }
}

impl SessionContinuation for StageSelection {
    fn reactions(&self) -> &[String] {
        &self.reactions
    }

    fn resume(self: Box<Self>, context: SessionContext, reaction: String) -> SessionFuture {
        Box::pin(async move {
            let Some(index) = self.reactions.iter().position(|emoji| emoji == &reaction) else {
                return Ok(SessionResume::complete(Vec::new()));
            };
            let entry = &self.matches[index];
            Ok(SessionResume::complete(vec![
                CoreActionData::EditMessage {
                    channel_id: context.channel_id().to_owned(),
                    message_id: context.message_id().to_owned(),
                    content: self.format(&format!("選択済み: {}", format_label(entry))),
                },
                CoreActionData::SendMessage {
                    channel_id: context.channel_id().to_owned(),
                    content: format_detail(entry, false),
                },
            ]))
        })
    }

    fn timeout_content(&self) -> Option<String> {
        Some(self.format("選択受付は終了しました。"))
    }
}

struct StagePages {
    query: String,
    matches: Arc<Vec<Arc<StageEntry>>>,
    page_index: usize,
    reactions: Vec<String>,
}

impl StagePages {
    fn new(query: String, matches: Arc<Vec<Arc<StageEntry>>>, page_index: usize) -> Self {
        let page_count = matches.len().div_ceil(PAGE_SIZE);
        let mut reactions = Vec::new();
        if page_index > 0 {
            reactions.push(PREVIOUS_PAGE_EMOJI.to_owned());
        }
        if page_index + 1 < page_count {
            reactions.push(NEXT_PAGE_EMOJI.to_owned());
        }
        Self {
            query,
            matches,
            page_index,
            reactions,
        }
    }

    fn format(&self, footer: &str) -> String {
        format_page(&self.query, &self.matches, self.page_index, footer, true)
    }
}

impl SessionContinuation for StagePages {
    fn reactions(&self) -> &[String] {
        &self.reactions
    }

    fn resume(self: Box<Self>, context: SessionContext, reaction: String) -> SessionFuture {
        Box::pin(async move {
            let page_index = if reaction == PREVIOUS_PAGE_EMOJI {
                self.page_index.saturating_sub(1)
            } else {
                self.page_index + 1
            };
            let next = StagePages::new(self.query.clone(), Arc::clone(&self.matches), page_index);
            let content = next.format(LIST_FOOTER);
            Ok(SessionResume::continue_with(
                vec![CoreActionData::EditMessage {
                    channel_id: context.channel_id().to_owned(),
                    message_id: context.message_id().to_owned(),
                    content,
                }],
                Box::new(next),
            ))
        })
    }

    fn timeout_content(&self) -> Option<String> {
        Some(self.format("ページ操作受付は終了しました。詳細は o.st <ID> で表示できます。"))
    }
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}
