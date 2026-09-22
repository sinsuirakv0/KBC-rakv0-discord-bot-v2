//! 敵ユニット検索・画像・関連Fileを扱う`tut` Command。

mod data_source;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kbc_protocol::CoreActionData;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::commands::common::file_picker::{
    AssetFileOption, NEXT_PAGE_EMOJI, NUMBER_EMOJIS, PREVIOUS_PAGE_EMOJI, file_picker,
};
use crate::commands::common::search::normalize_search_text;
use crate::motion::request::{MotionRequest, parse_motion_arguments};
use crate::motion::{MotionJob, MotionPlan};
use crate::services::HttpService;
use crate::session::{
    SessionContext, SessionContinuation, SessionFuture, SessionRequest, SessionResume,
};
use crate::task_runtime::{TaskRuntime, TaskSubmission, TaskSubmitError};

use data_source::TutDataSource;

const SEARCH_PAGE_URL: &str = "https://jarjarblink.github.io/JDB/tunit_search.html?cc=ja";
const DETAIL_PAGE_URL: &str = "https://jarjarblink.github.io/JDB/t000.html?cc=ja";
const REACTION_TIMEOUT: Duration = Duration::from_secs(60);
const PAGE_SIZE: usize = 20;
const NOT_FOUND_MESSAGE: &str = "該当する敵ユニットが見つかりませんでした。";
const DATA_ERROR_MESSAGE: &str = "敵データを取得できませんでした。時間をおいて再度お試しください。";
const IMAGE_ERROR_MESSAGE: &str = "敵画像の取得に失敗しました。";
const INVALID_FILE_MESSAGE: &str =
    "fileの指定が正しくありません。o.tut help で使い方を確認してください。";
const MISSING_FILE_MESSAGE: &str = "この敵に関連するファイルが見つかりませんでした。";
const MISSING_MOTION_MESSAGE: &str = "この敵のモーション素材が見つかりませんでした。";
const MOTION_BUSY_MESSAGE: &str =
    "現在ほかのモーションを処理中です。完了してからもう一度お試しください。";
const INVALID_MOTION_MESSAGE: &str =
    "motionの指定が正しくありません。o.tut help で使い方を確認してください。";
const LIST_FOOTER: &str = "詳細は o.tut <ID> で表示できます。";

#[derive(Clone)]
struct EnemySearchEntry {
    id: usize,
    display_name: String,
    aliases: Vec<String>,
}

struct EnemySearchData {
    entries: Vec<EnemySearchEntry>,
    id_index: HashMap<usize, usize>,
}

#[derive(Clone)]
struct TutSearchMatch {
    enemy: EnemySearchEntry,
    matched_alias: Option<String>,
}

#[derive(Clone)]
enum TutOperation {
    Detail,
    Origin,
    File,
    Motion(MotionRequest),
}

enum TutRequest {
    Landing,
    Help,
    InvalidFile,
    InvalidMotion,
    Search {
        query: String,
        force: bool,
        operation: TutOperation,
    },
}

pub(super) struct TutCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: TutDataSource,
    tasks: Arc<TaskRuntime>,
    ffmpeg_path: Option<PathBuf>,
}

impl TutCommand {
    pub(super) fn new(
        help: &str,
        http: Arc<HttpService>,
        tasks: Arc<TaskRuntime>,
        ffmpeg_path: Option<PathBuf>,
    ) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "tut".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            data_source: TutDataSource::new(http),
            tasks,
            ffmpeg_path,
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
            return Ok(message(channel_id, self.help.clone()));
        }
        let (query, force, operation) = match parse_request(arguments) {
            TutRequest::Landing => return Ok(message(channel_id, SEARCH_PAGE_URL)),
            TutRequest::Help => return Ok(message(channel_id, self.help.clone())),
            TutRequest::InvalidFile => return Ok(message(channel_id, INVALID_FILE_MESSAGE)),
            TutRequest::InvalidMotion => return Ok(message(channel_id, INVALID_MOTION_MESSAGE)),
            TutRequest::Search {
                query,
                force,
                operation,
            } => (query, force, operation),
        };

        let data = match self.data_source.fetch_search_data().await {
            Ok(data) => data,
            Err(error) => {
                eprintln!("Enemy data retrieval failed: {error}");
                return Ok(message(channel_id, DATA_ERROR_MESSAGE));
            }
        };
        let matches = search_enemies(&data, &query, force);
        if matches.is_empty() {
            return Ok(message(channel_id, NOT_FOUND_MESSAGE));
        }

        if matches.len() == 1 && !matches!(&operation, TutOperation::Detail) {
            return self
                .run_selected(context, matches.into_iter().next().unwrap(), operation)
                .await;
        }
        if matches.len() <= 3 && matches!(&operation, TutOperation::Detail) {
            let mut output = CommandOutput::new();
            for matched in matches {
                output.push(CoreActionData::SendMessage {
                    channel_id: channel_id.to_owned(),
                    content: format_detail(&matched),
                })?;
            }
            return Ok(output);
        }
        if matches.len() <= NUMBER_EMOJIS.len() {
            let continuation = TutSelection::new(
                query.clone(),
                matches.clone(),
                operation,
                self.data_source.clone(),
                Arc::clone(&self.tasks),
                self.ffmpeg_path.clone(),
            );
            let content = continuation.format("数字のリアクションで選択してください。");
            let session = SessionRequest::new(
                context.user_id().to_owned(),
                channel_id.to_owned(),
                REACTION_TIMEOUT,
                Box::new(continuation),
            )?
            .clear_reactions_on_completion();
            return Ok(CommandOutput::with_session(
                CoreActionData::SendMessage {
                    channel_id: channel_id.to_owned(),
                    content,
                },
                session,
            ));
        }
        list_output(context, query, matches)
    }

    async fn run_selected(
        &self,
        context: &CommandContext,
        matched: TutSearchMatch,
        operation: TutOperation,
    ) -> Result<CommandOutput, crate::command::CommandExecutionError> {
        match operation {
            TutOperation::Detail => Ok(message(context.channel_id(), format_detail(&matched))),
            TutOperation::Origin => {
                let action = fetch_origin(context.channel_id(), &matched, &self.data_source).await;
                Ok(CommandOutput::single(action))
            }
            TutOperation::File => file_output(context, &matched, &self.data_source).await,
            TutOperation::Motion(request) => {
                motion_output(
                    context.channel_id(),
                    context.request_id(),
                    &matched,
                    request,
                    &self.data_source,
                    &self.tasks,
                    self.ffmpeg_path.as_deref(),
                )
                .await
            }
        }
    }
}

impl Command for TutCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(&context, &arguments).await })
    }
}

fn parse_request(arguments: &[String]) -> TutRequest {
    if arguments.is_empty() {
        return TutRequest::Landing;
    }
    let mut force = false;
    let mut origin = false;
    let mut file = false;
    let mut motion = false;
    let mut query_parts = Vec::new();
    let mut file_parts = Vec::new();
    let mut motion_parts = Vec::new();
    for argument in arguments {
        let normalized = argument.to_ascii_lowercase();
        if normalized == "-f" || normalized == "-force" {
            force = true;
        } else if normalized == "origin" {
            origin = true;
        } else if normalized == "file" && !file {
            file = true;
        } else if normalized == "motion" && !motion {
            motion = true;
        } else if file {
            file_parts.push(argument.clone());
        } else if motion {
            motion_parts.push(argument.clone());
        } else if !motion {
            query_parts.push(argument.clone());
        }
    }
    let query = query_parts.join(" ").trim().to_owned();
    if query.is_empty() {
        return TutRequest::Help;
    }
    if file {
        if origin || motion || !file_parts.is_empty() {
            return TutRequest::InvalidFile;
        }
        return TutRequest::Search {
            query,
            force,
            operation: TutOperation::File,
        };
    }
    if motion {
        if origin {
            return TutRequest::InvalidMotion;
        }
        let Some(request) = parse_motion_arguments(&motion_parts, &[]) else {
            return TutRequest::InvalidMotion;
        };
        return TutRequest::Search {
            query,
            force,
            operation: TutOperation::Motion(request),
        };
    }
    TutRequest::Search {
        query,
        force,
        operation: if origin {
            TutOperation::Origin
        } else {
            TutOperation::Detail
        },
    }
}

fn search_enemies(data: &EnemySearchData, query: &str, force: bool) -> Vec<TutSearchMatch> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.chars().all(|character| character.is_ascii_digit())
        && let Ok(id) = trimmed.parse::<usize>()
        && let Some(index) = data.id_index.get(&id)
    {
        return vec![TutSearchMatch {
            enemy: data.entries[*index].clone(),
            matched_alias: None,
        }];
    }
    let searchable_query = if force {
        trimmed.to_owned()
    } else {
        normalize_search_text(trimmed)
    };
    let words = searchable_query.split_whitespace().collect::<Vec<_>>();
    let mut matches = Vec::new();
    for enemy in &data.entries {
        if name_matches(&enemy.display_name, &words, force) {
            matches.push(TutSearchMatch {
                enemy: enemy.clone(),
                matched_alias: None,
            });
            continue;
        }
        if let Some(alias) = enemy
            .aliases
            .iter()
            .find(|alias| name_matches(alias, &words, force))
        {
            matches.push(TutSearchMatch {
                enemy: enemy.clone(),
                matched_alias: Some(alias.clone()),
            });
        }
    }
    matches
}

fn name_matches(name: &str, words: &[&str], force: bool) -> bool {
    let searchable = if force {
        name.to_owned()
    } else {
        normalize_search_text(name)
    };
    words.iter().all(|word| searchable.contains(word))
}

fn display_name(matched: &TutSearchMatch) -> String {
    if matched.enemy.display_name != "ダミー" {
        return matched.enemy.display_name.clone();
    }
    let alias = matched.matched_alias.as_deref().or_else(|| {
        matched
            .enemy
            .aliases
            .iter()
            .find(|name| *name != "ダミー")
            .map(String::as_str)
    });
    alias
        .map(|name| format!("{name} (ダミー)"))
        .unwrap_or_else(|| "ダミー".to_owned())
}

fn format_label(matched: &TutSearchMatch) -> String {
    format!("{} {}", matched.enemy.id, display_name(matched))
}

fn format_detail(matched: &TutSearchMatch) -> String {
    format!(
        "{}\n{}&unit={}",
        format_label(matched),
        DETAIL_PAGE_URL,
        matched.enemy.id
    )
}

fn format_selection(query: &str, matches: &[TutSearchMatch], footer: &str) -> String {
    let lines = matches
        .iter()
        .enumerate()
        .map(|(index, matched)| format!("{} {}", NUMBER_EMOJIS[index], format_label(matched)))
        .collect::<Vec<_>>();
    let count = matches.len();
    let lines = lines.join("\n");
    format!("敵ユニット「{query}」検索結果 1～{count}/{count}\n```text\n{lines}\n```\n{footer}")
}

fn format_page(
    query: &str,
    matches: &[TutSearchMatch],
    page_index: usize,
    footer: &str,
    include_page_count: bool,
) -> String {
    let start = page_index * PAGE_SIZE;
    let end = matches.len().min(start + PAGE_SIZE);
    let page = matches[start..end]
        .iter()
        .map(format_label)
        .collect::<Vec<_>>();
    let pages = if include_page_count {
        format!(
            "（{}/{}ページ）",
            page_index + 1,
            matches.len().div_ceil(PAGE_SIZE)
        )
    } else {
        String::new()
    };
    format!(
        "敵ユニット「{query}」検索結果 {}～{}/{}{}\n```text\n{}\n```\n{}",
        start + 1,
        end,
        matches.len(),
        pages,
        page.join("\n"),
        footer
    )
}

fn list_output(
    context: &CommandContext,
    query: String,
    matches: Vec<TutSearchMatch>,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    if matches.len() <= PAGE_SIZE {
        return Ok(message(
            context.channel_id(),
            format_page(&query, &matches, 0, LIST_FOOTER, false),
        ));
    }
    let continuation = TutPages::new(query.clone(), Arc::new(matches), 0);
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

async fn fetch_origin(
    channel_id: &str,
    matched: &TutSearchMatch,
    data_source: &TutDataSource,
) -> CoreActionData {
    let relative_path = format!("Image/enemy_icon_{:03}.png", matched.enemy.id);
    match data_source
        .assets()
        .attachment(channel_id, &relative_path)
        .await
    {
        Ok(action) => action,
        Err(error) => {
            eprintln!("Enemy image retrieval failed: {error}");
            CoreActionData::SendMessage {
                channel_id: channel_id.to_owned(),
                content: IMAGE_ERROR_MESSAGE.to_owned(),
            }
        }
    }
}

async fn prepare_file_picker(
    matched: &TutSearchMatch,
    data_source: &TutDataSource,
) -> Result<Option<(String, Box<dyn SessionContinuation>)>, String> {
    let id = format!("{:03}", matched.enemy.id);
    let base = format!("{id}_e");
    let candidates = [
        (format!("Image/enemy_icon_{id}.png"), "敵アイコン"),
        (format!("Number/{base}.png"), "スプライト"),
        (format!("ImageData/{base}.imgcut"), "切り抜き情報"),
        (format!("ImageData/{base}.mamodel"), "モデル"),
        (format!("ImageData/{base}00.maanim"), "歩行"),
        (format!("ImageData/{base}01.maanim"), "待機"),
        (format!("ImageData/{base}02.maanim"), "攻撃"),
        (format!("ImageData/{base}03.maanim"), "ノックバック"),
    ];
    let paths = candidates
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    let assets = data_source.assets();
    let existing = assets
        .find_existing(&paths)
        .await
        .map_err(|error| error.to_string())?;
    let options = candidates
        .into_iter()
        .filter(|(path, _)| existing.contains(path))
        .map(|(relative_path, label)| AssetFileOption {
            relative_path,
            label: label.to_owned(),
        })
        .collect::<Vec<_>>();
    if options.is_empty() {
        return Ok(None);
    }
    Ok(Some(file_picker(
        format!(
            "敵ユニット「{} {}」関連ファイル",
            matched.enemy.id, matched.enemy.display_name
        ),
        options,
        assets,
        DATA_ERROR_MESSAGE,
    )))
}

async fn file_output(
    context: &CommandContext,
    matched: &TutSearchMatch,
    data_source: &TutDataSource,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    match prepare_file_picker(matched, data_source).await {
        Ok(Some((content, continuation))) => {
            let session = SessionRequest::new(
                context.user_id().to_owned(),
                context.channel_id().to_owned(),
                REACTION_TIMEOUT,
                continuation,
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
        Ok(None) => Ok(message(context.channel_id(), MISSING_FILE_MESSAGE)),
        Err(error) => {
            eprintln!("Enemy file check failed: {error}");
            Ok(message(context.channel_id(), DATA_ERROR_MESSAGE))
        }
    }
}

struct TutSelection {
    query: String,
    matches: Vec<TutSearchMatch>,
    operation: TutOperation,
    reactions: Vec<String>,
    data_source: TutDataSource,
    tasks: Arc<TaskRuntime>,
    ffmpeg_path: Option<PathBuf>,
}

impl TutSelection {
    fn new(
        query: String,
        matches: Vec<TutSearchMatch>,
        operation: TutOperation,
        data_source: TutDataSource,
        tasks: Arc<TaskRuntime>,
        ffmpeg_path: Option<PathBuf>,
    ) -> Self {
        let reactions = NUMBER_EMOJIS[..matches.len()]
            .iter()
            .map(|emoji| (*emoji).to_owned())
            .collect();
        Self {
            query,
            matches,
            operation,
            reactions,
            data_source,
            tasks,
            ffmpeg_path,
        }
    }

    fn format(&self, footer: &str) -> String {
        format_selection(&self.query, &self.matches, footer)
    }
}

impl SessionContinuation for TutSelection {
    fn reactions(&self) -> &[String] {
        &self.reactions
    }

    fn resume(self: Box<Self>, context: SessionContext, reaction: String) -> SessionFuture {
        Box::pin(async move {
            let Some(index) = self.reactions.iter().position(|emoji| emoji == &reaction) else {
                return Ok(SessionResume::complete(Vec::new()));
            };
            let matched = &self.matches[index];
            let mut actions = vec![CoreActionData::EditMessage {
                channel_id: context.channel_id().to_owned(),
                message_id: context.message_id().to_owned(),
                content: self.format(&format!("選択済み: {}", format_label(matched))),
            }];
            match self.operation.clone() {
                TutOperation::Detail => {
                    actions.push(CoreActionData::SendMessage {
                        channel_id: context.channel_id().to_owned(),
                        content: format_detail(matched),
                    });
                    Ok(SessionResume::complete(actions))
                }
                TutOperation::Origin => {
                    actions
                        .push(fetch_origin(context.channel_id(), matched, &self.data_source).await);
                    Ok(SessionResume::complete(actions))
                }
                TutOperation::File => match prepare_file_picker(matched, &self.data_source).await {
                    Ok(Some((content, continuation))) => {
                        actions.push(CoreActionData::SendMessage {
                            channel_id: context.channel_id().to_owned(),
                            content,
                        });
                        Ok(SessionResume::start_new(actions, continuation))
                    }
                    Ok(None) => {
                        actions.push(CoreActionData::SendMessage {
                            channel_id: context.channel_id().to_owned(),
                            content: MISSING_FILE_MESSAGE.to_owned(),
                        });
                        Ok(SessionResume::complete(actions))
                    }
                    Err(error) => {
                        eprintln!("Enemy file check failed: {error}");
                        actions.push(CoreActionData::SendMessage {
                            channel_id: context.channel_id().to_owned(),
                            content: DATA_ERROR_MESSAGE.to_owned(),
                        });
                        Ok(SessionResume::complete(actions))
                    }
                },
                TutOperation::Motion(request) => {
                    let output = motion_output(
                        context.channel_id(),
                        context.request_id(),
                        matched,
                        request,
                        &self.data_source,
                        &self.tasks,
                        self.ffmpeg_path.as_deref(),
                    )
                    .await?;
                    actions.extend(output.into_actions());
                    Ok(SessionResume::complete(actions))
                }
            }
        })
    }

    fn timeout_content(&self) -> Option<String> {
        Some(self.format("選択受付は終了しました。"))
    }
}

struct TutPages {
    query: String,
    matches: Arc<Vec<TutSearchMatch>>,
    page_index: usize,
    reactions: Vec<String>,
}

impl TutPages {
    fn new(query: String, matches: Arc<Vec<TutSearchMatch>>, page_index: usize) -> Self {
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

impl SessionContinuation for TutPages {
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
            let next = TutPages::new(self.query.clone(), Arc::clone(&self.matches), page_index);
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
        Some(self.format("ページ操作受付は終了しました。詳細は o.tut <ID> で表示できます。"))
    }
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}

fn resolve_motion_plan(matched: &TutSearchMatch, request: MotionRequest) -> MotionPlan {
    let asset_id = format!("{:03}", matched.enemy.id);
    let mut animation_paths = HashMap::new();
    for segment in &request.segments {
        animation_paths.entry(segment.motion).or_insert_with(|| {
            format!(
                "ImageData/{asset_id}_e0{}.maanim",
                segment.motion.asset_index()
            )
        });
    }
    MotionPlan {
        format: request.format,
        full: request.full,
        filename_stem: format!("tut-{asset_id}-motion"),
        preview_scale: if matched.enemy.id == 0 { 2.25 } else { 1.0 },
        segments: request.segments,
        sprite_path: format!("Number/{asset_id}_e.png"),
        imgcut_path: format!("ImageData/{asset_id}_e.imgcut"),
        model_path: format!("ImageData/{asset_id}_e.mamodel"),
        animation_paths,
    }
}

#[allow(clippy::too_many_arguments)]
async fn motion_output(
    channel_id: &str,
    request_id: &kbc_protocol::RequestId,
    matched: &TutSearchMatch,
    request: MotionRequest,
    data_source: &TutDataSource,
    tasks: &TaskRuntime,
    ffmpeg_path: Option<&std::path::Path>,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let plan = resolve_motion_plan(matched, request);
    let mut paths = vec![
        plan.sprite_path.clone(),
        plan.imgcut_path.clone(),
        plan.model_path.clone(),
    ];
    paths.extend(plan.animation_paths.values().cloned());
    let assets = data_source.assets();
    let existing = match assets.find_existing(&paths).await {
        Ok(existing) => existing,
        Err(error) => {
            eprintln!("Enemy motion asset check failed: {error}");
            return Ok(message(channel_id, DATA_ERROR_MESSAGE));
        }
    };
    if paths.iter().any(|path| !existing.contains(path)) {
        return Ok(message(channel_id, MISSING_MOTION_MESSAGE));
    }
    let job = MotionJob::new(plan, assets, ffmpeg_path.map(PathBuf::from));
    match tasks.submit(TaskSubmission::new(
        request_id.clone(),
        channel_id.to_owned(),
        format!(
            "⏳ 敵 {} {} のモーション生成を受け付けました",
            matched.enemy.id,
            display_name(matched)
        ),
        Box::new(job),
    )) {
        Ok(_) => Ok(CommandOutput::new()),
        Err(TaskSubmitError::Busy | TaskSubmitError::Unavailable) => {
            Ok(message(channel_id, MOTION_BUSY_MESSAGE))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_non_motion_requests_and_normalized_search() {
        assert!(matches!(parse_request(&[]), TutRequest::Landing));
        assert!(matches!(
            parse_request(&["origin".to_owned()]),
            TutRequest::Help
        ));
        assert!(matches!(
            parse_request(&["わんこ".to_owned(), "file".to_owned()]),
            TutRequest::Search {
                operation: TutOperation::File,
                ..
            }
        ));

        let data = EnemySearchData {
            entries: vec![EnemySearchEntry {
                id: 1,
                display_name: "ワンコ".to_owned(),
                aliases: vec!["犬".to_owned()],
            }],
            id_index: HashMap::from([(1, 0)]),
        };
        assert_eq!(search_enemies(&data, "わんこ", false).len(), 1);
        assert_eq!(search_enemies(&data, "1", false)[0].enemy.id, 1);
    }
}
