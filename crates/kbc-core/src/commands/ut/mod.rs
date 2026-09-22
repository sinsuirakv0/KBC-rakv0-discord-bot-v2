//! 味方キャラ検索・画像・関連Fileを扱う`ut` Command。

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

use data_source::UtDataSource;

const SEARCH_PAGE_URL: &str = "https://jarjarblink.github.io/JDB/unit_search.html?cc=ja";
const DETAIL_PAGE_BASE_URL: &str = "https://jarjarblink.github.io/JDB";
const REACTION_TIMEOUT: Duration = Duration::from_secs(60);
const PAGE_SIZE: usize = 20;
const NOT_FOUND_MESSAGE: &str = "該当する味方キャラが見つかりませんでした。";
const INVALID_ORIGIN_MESSAGE: &str =
    "originの指定が正しくありません。o.ut help で使い方を確認してください。";
const INVALID_FILE_MESSAGE: &str =
    "fileの指定が正しくありません。o.ut help で使い方を確認してください。";
const INVALID_MOTION_MESSAGE: &str =
    "motionの指定が正しくありません。o.ut help で使い方を確認してください。";
const MISSING_IMAGE_MESSAGE: &str = "指定した画像はこのキャラには存在しません。";
const MISSING_FILE_MESSAGE: &str = "このキャラに関連するファイルが見つかりませんでした。";
const MISSING_MOTION_MESSAGE: &str = "このキャラのモーション素材が見つかりませんでした。";
const MOTION_BUSY_MESSAGE: &str =
    "現在ほかのモーションを処理中です。完了してからもう一度お試しください。";
const DATA_ERROR_MESSAGE: &str =
    "味方キャラデータの取得に失敗しました。しばらくしてからもう一度お試しください。";
const LIST_FOOTER: &str = "詳細は o.ut <ID> で表示できます。";

#[derive(Clone)]
struct CharacterForm {
    name: String,
}

#[derive(Clone)]
struct CharacterUnit {
    id: String,
    forms: Vec<CharacterForm>,
    aliases: Vec<String>,
}

struct CharacterIndex {
    units: Vec<CharacterUnit>,
}

struct UnitBuyEntry {
    id: String,
    shared_form_ids: [Option<String>; 2],
}

struct UnitBuy {
    units: Vec<UnitBuyEntry>,
}

#[derive(Clone, Copy)]
enum MatchSource {
    Id,
    Form(usize),
    Alias,
}

#[derive(Clone)]
struct UtSearchMatch {
    unit: CharacterUnit,
    source: MatchSource,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum UtForm {
    First,
    Second,
    Third,
    Fourth,
}

impl UtForm {
    const ALL: [Self; 4] = [Self::First, Self::Second, Self::Third, Self::Fourth];

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "f" => Some(Self::First),
            "c" => Some(Self::Second),
            "s" => Some(Self::Third),
            "u" => Some(Self::Fourth),
            _ => None,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::First => "f",
            Self::Second => "c",
            Self::Third => "s",
            Self::Fourth => "u",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::First => "第一形態",
            Self::Second => "第二形態",
            Self::Third => "第三形態",
            Self::Fourth => "第四形態",
        }
    }
}

#[derive(Clone, Copy)]
enum OriginFamily {
    Icon,
    Wide,
    Gacha,
    Sprite,
}

#[derive(Clone, Copy)]
enum OriginVariant {
    Form(UtForm),
    GachaMiddle,
    GachaThird,
}

#[derive(Clone, Copy)]
struct OriginRequest {
    family: OriginFamily,
    variant: OriginVariant,
}

#[derive(Clone, Copy)]
struct FileRequest {
    form: Option<UtForm>,
}

#[derive(Clone)]
enum UtOperation {
    Detail,
    Origin(OriginRequest),
    File(FileRequest),
    Motion(MotionRequest),
}

enum UtRequest {
    Landing,
    InvalidOrigin,
    InvalidFile,
    InvalidMotion,
    Search {
        query: String,
        force: bool,
        operation: UtOperation,
    },
}

struct UnitAssetStem {
    asset_id: String,
    suffix: &'static str,
    shared: bool,
}

pub(super) struct UtCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: UtDataSource,
    tasks: Arc<TaskRuntime>,
    ffmpeg_path: Option<PathBuf>,
}

impl UtCommand {
    pub(super) fn new(
        help: &str,
        http: Arc<HttpService>,
        tasks: Arc<TaskRuntime>,
        ffmpeg_path: Option<PathBuf>,
    ) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "ut".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            data_source: UtDataSource::new(http),
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
            UtRequest::Landing => return Ok(message(channel_id, SEARCH_PAGE_URL)),
            UtRequest::InvalidOrigin => return Ok(message(channel_id, INVALID_ORIGIN_MESSAGE)),
            UtRequest::InvalidFile => return Ok(message(channel_id, INVALID_FILE_MESSAGE)),
            UtRequest::InvalidMotion => return Ok(message(channel_id, INVALID_MOTION_MESSAGE)),
            UtRequest::Search {
                query,
                force,
                operation,
            } => (query, force, operation),
        };

        let index = match self.data_source.fetch_character_index().await {
            Ok(index) => index,
            Err(error) => {
                eprintln!("Character index retrieval failed: {error}");
                return Ok(message(channel_id, DATA_ERROR_MESSAGE));
            }
        };
        let matches = search_character_index(&index, &query, force);
        if matches.is_empty() {
            return Ok(message(channel_id, NOT_FOUND_MESSAGE));
        }

        if matches.len() == 1 && !matches!(&operation, UtOperation::Detail) {
            return self.run_selected(context, &matches[0], operation).await;
        }
        if matches.len() <= 3 && matches!(&operation, UtOperation::Detail) {
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
            let continuation = UtSelection::new(
                query.clone(),
                matches,
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
        matched: &UtSearchMatch,
        operation: UtOperation,
    ) -> Result<CommandOutput, crate::command::CommandExecutionError> {
        match operation {
            UtOperation::Detail => Ok(message(context.channel_id(), format_detail(matched))),
            UtOperation::Origin(request) => Ok(CommandOutput::single(
                fetch_origin(context.channel_id(), matched, request, &self.data_source).await,
            )),
            UtOperation::File(request) => {
                file_output(context, matched, request, &self.data_source).await
            }
            UtOperation::Motion(request) => {
                motion_output(
                    context.channel_id(),
                    context.request_id(),
                    matched,
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

impl Command for UtCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(&context, &arguments).await })
    }
}

fn parse_request(arguments: &[String]) -> UtRequest {
    if arguments.is_empty() {
        return UtRequest::Landing;
    }
    let normalized = arguments
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let origin_index = normalized.iter().position(|value| value == "origin");
    let file_index = normalized.iter().position(|value| value == "file");
    let motion_index = normalized.iter().position(|value| value == "motion");
    let mut operations = [origin_index, file_index, motion_index]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    operations.sort_unstable();
    if operations.len() > 1 {
        let first = operations[0];
        return if Some(first) == file_index {
            UtRequest::InvalidFile
        } else if Some(first) == motion_index {
            UtRequest::InvalidMotion
        } else {
            UtRequest::InvalidOrigin
        };
    }

    let operation_index = operations.first().copied();
    let query_arguments = operation_index
        .map(|index| &arguments[..index])
        .unwrap_or(arguments);
    let force = query_arguments
        .iter()
        .any(|value| value.eq_ignore_ascii_case("-f"));
    let query = query_arguments
        .iter()
        .filter(|value| !value.eq_ignore_ascii_case("-f"))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned();
    let Some(operation_index) = operation_index else {
        return UtRequest::Search {
            query,
            force,
            operation: UtOperation::Detail,
        };
    };
    if query.is_empty() {
        return if Some(operation_index) == file_index {
            UtRequest::InvalidFile
        } else if Some(operation_index) == motion_index {
            UtRequest::InvalidMotion
        } else {
            UtRequest::InvalidOrigin
        };
    }
    if Some(operation_index) == motion_index {
        let Some(request) =
            parse_motion_arguments(&arguments[operation_index + 1..], &["f", "c", "s", "u"])
        else {
            return UtRequest::InvalidMotion;
        };
        return UtRequest::Search {
            query,
            force,
            operation: UtOperation::Motion(request),
        };
    }
    if Some(operation_index) == file_index {
        let file_arguments = &arguments[operation_index + 1..];
        let form = match file_arguments {
            [] => None,
            [value] => match UtForm::parse(value) {
                Some(form) => Some(form),
                None => return UtRequest::InvalidFile,
            },
            _ => return UtRequest::InvalidFile,
        };
        return UtRequest::Search {
            query,
            force,
            operation: UtOperation::File(FileRequest { form }),
        };
    }
    match parse_origin(&arguments[operation_index + 1..]) {
        Some(request) => UtRequest::Search {
            query,
            force,
            operation: UtOperation::Origin(request),
        },
        None => UtRequest::InvalidOrigin,
    }
}

fn parse_origin(arguments: &[String]) -> Option<OriginRequest> {
    let values = arguments
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Some(OriginRequest {
            family: OriginFamily::Icon,
            variant: OriginVariant::Form(UtForm::First),
        });
    }
    if values.len() == 1
        && let Some(form) = UtForm::parse(&values[0])
    {
        return Some(OriginRequest {
            family: OriginFamily::Icon,
            variant: OriginVariant::Form(form),
        });
    }
    let family = match values[0].as_str() {
        "icon" => OriginFamily::Icon,
        "wide" => OriginFamily::Wide,
        "sprite" => OriginFamily::Sprite,
        "gacha" => OriginFamily::Gacha,
        _ => return None,
    };
    if matches!(family, OriginFamily::Gacha) {
        return match values.as_slice() {
            [_] => Some(OriginRequest {
                family,
                variant: OriginVariant::Form(UtForm::First),
            }),
            [_, variant] if variant == "m" => Some(OriginRequest {
                family,
                variant: OriginVariant::GachaMiddle,
            }),
            [_, variant] if variant == "z" => Some(OriginRequest {
                family,
                variant: OriginVariant::GachaThird,
            }),
            _ => None,
        };
    }
    let form = match values.as_slice() {
        [_] => UtForm::First,
        [_, value] => UtForm::parse(value)?,
        _ => return None,
    };
    Some(OriginRequest {
        family,
        variant: OriginVariant::Form(form),
    })
}

fn search_character_index(index: &CharacterIndex, query: &str, force: bool) -> Vec<UtSearchMatch> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if !force
        && trimmed.chars().all(|character| character.is_ascii_digit())
        && let Ok(id) = trimmed.parse::<usize>()
        && let Some(unit) = index.units.get(id)
        && unit.id.parse::<usize>() == Ok(id)
    {
        return vec![UtSearchMatch {
            unit: unit.clone(),
            source: MatchSource::Id,
        }];
    }
    let searchable_query = if force {
        trimmed.to_owned()
    } else {
        normalize_search_text(trimmed)
    };
    let words = searchable_query.split_whitespace().collect::<Vec<_>>();
    let mut matches = Vec::new();
    for unit in &index.units {
        let form_match = unit.forms.iter().enumerate().find(|(_, form)| {
            let name = if force {
                form.name.clone()
            } else {
                normalize_search_text(&form.name)
            };
            words.iter().all(|word| name.contains(word))
        });
        if let Some((form_index, _)) = form_match {
            matches.push(UtSearchMatch {
                unit: unit.clone(),
                source: MatchSource::Form(form_index),
            });
            continue;
        }
        if !force
            && unit.aliases.iter().any(|alias| {
                let alias = normalize_search_text(alias);
                words.iter().all(|word| alias.contains(word))
            })
        {
            matches.push(UtSearchMatch {
                unit: unit.clone(),
                source: MatchSource::Alias,
            });
        }
    }
    matches.sort_by_key(|matched| matched.unit.id.parse::<usize>().unwrap_or(usize::MAX));
    matches
}

fn match_annotation(matched: &UtSearchMatch) -> &'static str {
    match matched.source {
        MatchSource::Alias => " (別称でヒット)",
        MatchSource::Form(1) => " (第二形態名でヒット)",
        MatchSource::Form(2) => " (第三形態名でヒット)",
        MatchSource::Form(3) => " (第四形態名でヒット)",
        MatchSource::Id | MatchSource::Form(_) => "",
    }
}

fn format_label(matched: &UtSearchMatch) -> String {
    format!(
        "{} {}{}",
        matched.unit.id,
        matched.unit.forms[0].name,
        match_annotation(matched)
    )
}

fn format_detail(matched: &UtSearchMatch) -> String {
    format!(
        "{}\n{}/u000.html?cc=ja&unit={}",
        format_label(matched),
        DETAIL_PAGE_BASE_URL,
        matched.unit.id
    )
}

fn format_selection(query: &str, matches: &[UtSearchMatch], footer: &str) -> String {
    let lines = matches
        .iter()
        .enumerate()
        .map(|(index, matched)| format!("{} {}", NUMBER_EMOJIS[index], format_label(matched)))
        .collect::<Vec<_>>()
        .join("\n");
    let count = matches.len();
    format!("味方キャラ「{query}」検索結果 1～{count}/{count}\n```text\n{lines}\n```\n{footer}")
}

fn format_page(
    query: &str,
    matches: &[UtSearchMatch],
    page_index: usize,
    footer: &str,
    include_page_count: bool,
) -> String {
    let start = page_index * PAGE_SIZE;
    let end = matches.len().min(start + PAGE_SIZE);
    let lines = matches[start..end]
        .iter()
        .map(format_label)
        .collect::<Vec<_>>()
        .join("\n");
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
        "味方キャラ「{query}」検索結果 {}～{}/{}{}\n```text\n{}\n```\n{}",
        start + 1,
        end,
        matches.len(),
        pages,
        lines,
        footer
    )
}

fn list_output(
    context: &CommandContext,
    query: String,
    matches: Vec<UtSearchMatch>,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    if matches.len() <= PAGE_SIZE {
        return Ok(message(
            context.channel_id(),
            format_page(&query, &matches, 0, LIST_FOOTER, false),
        ));
    }
    let continuation = UtPages::new(query.clone(), Arc::new(matches), 0);
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

fn resolve_asset_stem(unit_buy: &UnitBuy, id: &str, form: UtForm) -> Option<UnitAssetStem> {
    let index = id.parse::<usize>().ok()?;
    let entry = unit_buy.units.get(index)?;
    if entry.id != id {
        return None;
    }
    let shared_id = match form {
        UtForm::First => entry.shared_form_ids[0].as_ref(),
        UtForm::Second => entry.shared_form_ids[1].as_ref(),
        UtForm::Third | UtForm::Fourth => None,
    };
    Some(match shared_id {
        Some(shared_id) => UnitAssetStem {
            asset_id: shared_id.clone(),
            suffix: "m",
            shared: true,
        },
        None => UnitAssetStem {
            asset_id: id.to_owned(),
            suffix: form.suffix(),
            shared: false,
        },
    })
}

fn resolve_origin_path(
    unit_buy: Option<&UnitBuy>,
    id: &str,
    request: OriginRequest,
) -> Option<String> {
    if matches!(request.family, OriginFamily::Gacha) {
        let variant = match request.variant {
            OriginVariant::Form(UtForm::First) => "f",
            OriginVariant::GachaMiddle => "m",
            OriginVariant::GachaThird => "z",
            OriginVariant::Form(_) => return None,
        };
        return Some(format!("Image/gatyachara_{id}_{variant}.png"));
    }
    let OriginVariant::Form(form) = request.variant else {
        return None;
    };
    let stem = resolve_asset_stem(unit_buy?, id, form)?;
    if matches!(request.family, OriginFamily::Sprite) {
        return Some(format!("Number/{}_{}.png", stem.asset_id, stem.suffix));
    }
    let suffix = if stem.shared {
        format!("_m0{}.png", usize::from(form == UtForm::Second))
    } else if matches!(request.family, OriginFamily::Icon) {
        format!("_{}00.png", form.suffix())
    } else {
        format!("_{}.png", form.suffix())
    };
    let prefix = if matches!(request.family, OriginFamily::Icon) {
        "uni"
    } else {
        "udi"
    };
    Some(format!("Unit/{prefix}{}{suffix}", stem.asset_id))
}

async fn fetch_origin(
    channel_id: &str,
    matched: &UtSearchMatch,
    request: OriginRequest,
    data_source: &UtDataSource,
) -> CoreActionData {
    let unit_buy = if matches!(request.family, OriginFamily::Gacha) {
        None
    } else {
        match data_source.fetch_unit_buy().await {
            Ok(unit_buy) => Some(unit_buy),
            Err(error) => {
                eprintln!("UnitBuy retrieval failed: {error}");
                return send_message_action(channel_id, DATA_ERROR_MESSAGE);
            }
        }
    };
    let Some(relative_path) = resolve_origin_path(unit_buy.as_deref(), &matched.unit.id, request)
    else {
        return send_message_action(channel_id, MISSING_IMAGE_MESSAGE);
    };
    let assets = data_source.assets();
    let existing = match assets
        .find_existing(std::slice::from_ref(&relative_path))
        .await
    {
        Ok(existing) => existing,
        Err(error) => {
            eprintln!("Character image check failed: {error}");
            return send_message_action(channel_id, DATA_ERROR_MESSAGE);
        }
    };
    if !existing.contains(&relative_path) {
        return send_message_action(channel_id, MISSING_IMAGE_MESSAGE);
    }
    match assets.attachment(channel_id, &relative_path).await {
        Ok(action) => action,
        Err(error) => {
            eprintln!("Character image retrieval failed: {error}");
            send_message_action(channel_id, DATA_ERROR_MESSAGE)
        }
    }
}

fn resolve_file_options(
    unit: &CharacterUnit,
    unit_buy: &UnitBuy,
    form_filter: Option<UtForm>,
) -> Vec<AssetFileOption> {
    let mut options = Vec::new();
    for form in UtForm::ALL
        .into_iter()
        .take(unit.forms.len())
        .filter(|form| form_filter.is_none_or(|filter| *form == filter))
    {
        let Some(stem) = resolve_asset_stem(unit_buy, &unit.id, form) else {
            continue;
        };
        let shared_suffix = format!("_m0{}.png", usize::from(form == UtForm::Second));
        let icon_suffix = if stem.shared {
            shared_suffix.clone()
        } else {
            format!("_{}00.png", form.suffix())
        };
        let wide_suffix = if stem.shared {
            shared_suffix
        } else {
            format!("_{}.png", form.suffix())
        };
        let base = format!("{}_{}", stem.asset_id, stem.suffix);
        let label = form.label();
        options.extend([
            file_option(
                format!("Unit/uni{}{}", stem.asset_id, icon_suffix),
                format!("{label} アイコン"),
            ),
            file_option(
                format!("Unit/udi{}{}", stem.asset_id, wide_suffix),
                format!("{label} 横長画像"),
            ),
            file_option(format!("Number/{base}.png"), format!("{label} スプライト")),
            file_option(
                format!("ImageData/{base}.imgcut"),
                format!("{label} 切り抜き情報"),
            ),
            file_option(
                format!("ImageData/{base}.mamodel"),
                format!("{label} モデル"),
            ),
            file_option(
                format!("ImageData/{base}00.maanim"),
                format!("{label} 歩行"),
            ),
            file_option(
                format!("ImageData/{base}01.maanim"),
                format!("{label} 待機"),
            ),
            file_option(
                format!("ImageData/{base}02.maanim"),
                format!("{label} 攻撃"),
            ),
            file_option(
                format!("ImageData/{base}03.maanim"),
                format!("{label} ノックバック"),
            ),
        ]);
    }
    if form_filter.is_none() {
        for variant in ["f", "m", "z"] {
            options.push(file_option(
                format!("Image/gatyachara_{}_{}.png", unit.id, variant),
                format!("ガチャ画像 {variant}"),
            ));
        }
    }
    options
}

fn file_option(relative_path: String, label: String) -> AssetFileOption {
    AssetFileOption {
        relative_path,
        label,
    }
}

async fn prepare_file_picker(
    matched: &UtSearchMatch,
    request: FileRequest,
    data_source: &UtDataSource,
) -> Result<Option<(String, Box<dyn SessionContinuation>)>, String> {
    let unit_buy = data_source
        .fetch_unit_buy()
        .await
        .map_err(|error| error.to_string())?;
    let candidates = resolve_file_options(&matched.unit, &unit_buy, request.form);
    let paths = candidates
        .iter()
        .map(|option| option.relative_path.clone())
        .collect::<Vec<_>>();
    let assets = data_source.assets();
    let existing = assets
        .find_existing(&paths)
        .await
        .map_err(|error| error.to_string())?;
    let options = candidates
        .into_iter()
        .filter(|option| existing.contains(&option.relative_path))
        .collect::<Vec<_>>();
    if options.is_empty() {
        return Ok(None);
    }
    Ok(Some(file_picker(
        format!(
            "味方キャラ「{} {}」関連ファイル",
            matched.unit.id, matched.unit.forms[0].name
        ),
        options,
        assets,
        DATA_ERROR_MESSAGE,
    )))
}

async fn file_output(
    context: &CommandContext,
    matched: &UtSearchMatch,
    request: FileRequest,
    data_source: &UtDataSource,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    match prepare_file_picker(matched, request, data_source).await {
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
            eprintln!("Character file check failed: {error}");
            Ok(message(context.channel_id(), DATA_ERROR_MESSAGE))
        }
    }
}

struct UtSelection {
    query: String,
    matches: Vec<UtSearchMatch>,
    operation: UtOperation,
    reactions: Vec<String>,
    data_source: UtDataSource,
    tasks: Arc<TaskRuntime>,
    ffmpeg_path: Option<PathBuf>,
}

impl UtSelection {
    fn new(
        query: String,
        matches: Vec<UtSearchMatch>,
        operation: UtOperation,
        data_source: UtDataSource,
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

impl SessionContinuation for UtSelection {
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
                UtOperation::Detail => {
                    actions.push(send_message_action(
                        context.channel_id(),
                        format_detail(matched),
                    ));
                    Ok(SessionResume::complete(actions))
                }
                UtOperation::Origin(request) => {
                    actions.push(
                        fetch_origin(context.channel_id(), matched, request, &self.data_source)
                            .await,
                    );
                    Ok(SessionResume::complete(actions))
                }
                UtOperation::File(request) => {
                    match prepare_file_picker(matched, request, &self.data_source).await {
                        Ok(Some((content, continuation))) => {
                            actions.push(send_message_action(context.channel_id(), content));
                            Ok(SessionResume::start_new(actions, continuation))
                        }
                        Ok(None) => {
                            actions.push(send_message_action(
                                context.channel_id(),
                                MISSING_FILE_MESSAGE,
                            ));
                            Ok(SessionResume::complete(actions))
                        }
                        Err(error) => {
                            eprintln!("Character file check failed: {error}");
                            actions.push(send_message_action(
                                context.channel_id(),
                                DATA_ERROR_MESSAGE,
                            ));
                            Ok(SessionResume::complete(actions))
                        }
                    }
                }
                UtOperation::Motion(request) => {
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

struct UtPages {
    query: String,
    matches: Arc<Vec<UtSearchMatch>>,
    page_index: usize,
    reactions: Vec<String>,
}

impl UtPages {
    fn new(query: String, matches: Arc<Vec<UtSearchMatch>>, page_index: usize) -> Self {
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

impl SessionContinuation for UtPages {
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
            let next = UtPages::new(self.query.clone(), Arc::clone(&self.matches), page_index);
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
        Some(self.format("ページ操作受付は終了しました。詳細は o.ut <ID> で表示できます。"))
    }
}

fn send_message_action(channel_id: &str, content: impl Into<String>) -> CoreActionData {
    CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    }
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(send_message_action(channel_id, content))
}

fn resolve_motion_plan(
    unit_buy: &UnitBuy,
    matched: &UtSearchMatch,
    request: MotionRequest,
) -> Option<MotionPlan> {
    let form = request
        .form
        .as_deref()
        .and_then(UtForm::parse)
        .unwrap_or(UtForm::First);
    if !UtForm::ALL
        .into_iter()
        .take(matched.unit.forms.len())
        .any(|candidate| candidate == form)
    {
        return None;
    }
    let stem = resolve_asset_stem(unit_buy, &matched.unit.id, form)?;
    let mut animation_paths = HashMap::new();
    for segment in &request.segments {
        animation_paths.entry(segment.motion).or_insert_with(|| {
            format!(
                "ImageData/{}_{}0{}.maanim",
                stem.asset_id,
                stem.suffix,
                segment.motion.asset_index()
            )
        });
    }
    let base = format!("{}_{}", stem.asset_id, stem.suffix);
    Some(MotionPlan {
        format: request.format,
        full: request.full,
        filename_stem: format!("ut-{}-{}-motion", matched.unit.id, form.suffix()),
        preview_scale: match (matched.unit.id.as_str(), form) {
            ("000", UtForm::First) => 2.25,
            ("009", UtForm::First) => 0.82,
            _ => 1.0,
        },
        segments: request.segments,
        sprite_path: format!("Number/{base}.png"),
        imgcut_path: format!("ImageData/{base}.imgcut"),
        model_path: format!("ImageData/{base}.mamodel"),
        animation_paths,
    })
}

#[allow(clippy::too_many_arguments)]
async fn motion_output(
    channel_id: &str,
    request_id: &kbc_protocol::RequestId,
    matched: &UtSearchMatch,
    request: MotionRequest,
    data_source: &UtDataSource,
    tasks: &TaskRuntime,
    ffmpeg_path: Option<&std::path::Path>,
) -> Result<CommandOutput, crate::command::CommandExecutionError> {
    let unit_buy = match data_source.fetch_unit_buy().await {
        Ok(unit_buy) => unit_buy,
        Err(error) => {
            eprintln!("UnitBuy retrieval for motion failed: {error}");
            return Ok(message(channel_id, DATA_ERROR_MESSAGE));
        }
    };
    let Some(plan) = resolve_motion_plan(&unit_buy, matched, request) else {
        return Ok(message(channel_id, MISSING_MOTION_MESSAGE));
    };
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
            eprintln!("Character motion asset check failed: {error}");
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
            "⏳ {} {} のモーション生成を受け付けました",
            matched.unit.id, matched.unit.forms[0].name
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
    fn parses_search_requests_and_resolves_shared_assets() {
        assert!(matches!(parse_request(&[]), UtRequest::Landing));
        assert!(matches!(
            parse_request(&["ネコ".to_owned(), "file".to_owned(), "s".to_owned()]),
            UtRequest::Search {
                operation: UtOperation::File(FileRequest {
                    form: Some(UtForm::Third)
                }),
                ..
            }
        ));
        assert!(matches!(
            parse_request(&["ネコ".to_owned(), "motion".to_owned()]),
            UtRequest::InvalidMotion
        ));

        let unit_buy = UnitBuy {
            units: vec![UnitBuyEntry {
                id: "000".to_owned(),
                shared_form_ids: [Some("656".to_owned()), None],
            }],
        };
        let path = resolve_origin_path(
            Some(&unit_buy),
            "000",
            OriginRequest {
                family: OriginFamily::Icon,
                variant: OriginVariant::Form(UtForm::First),
            },
        );
        assert_eq!(path.as_deref(), Some("Unit/uni656_m00.png"));
    }
}
