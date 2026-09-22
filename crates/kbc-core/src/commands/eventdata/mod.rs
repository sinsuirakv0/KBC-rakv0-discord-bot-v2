//! 公式・KBCイベントデータのURLとFileを返す`eventdata` Command。

mod data_source;

use std::collections::HashMap;
use std::sync::Arc;

use kbc_protocol::CoreActionData;
use tokio::task::JoinSet;

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::services::{Clock, HttpService};

use data_source::{EventAttachment, EventDataSource, build_kbc_url};

const DEFAULT_TYPES: [EventDataType; 3] = [
    EventDataType::Gatya,
    EventDataType::Sale,
    EventDataType::Item,
];
const ALL_TYPES: [EventDataType; 5] = [
    EventDataType::Gatya,
    EventDataType::Sale,
    EventDataType::Item,
    EventDataType::Notice,
    EventDataType::Ad,
];
const INVALID_MESSAGE: &str =
    "❌ 指定が正しくありません。o.eventdata help で使い方を確認してください。";
const DATA_ERROR_MESSAGE: &str =
    "❌ eventdataの取得に失敗しました。時間をおいて再度お試しください。";

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum EventDataType {
    Sale,
    Gatya,
    Item,
    Ad,
    Notice,
}

impl EventDataType {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "sale" => Some(Self::Sale),
            "gatya" => Some(Self::Gatya),
            "item" => Some(Self::Item),
            "ad" => Some(Self::Ad),
            "notice" | "popup_notice" | "placement" => Some(Self::Notice),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sale => "sale",
            Self::Gatya => "gatya",
            Self::Item => "item",
            Self::Ad => "ad",
            Self::Notice => "notice",
        }
    }

    fn needs_jwt(self) -> bool {
        matches!(self, Self::Sale | Self::Gatya | Self::Item)
    }
}

#[derive(Clone, Copy)]
enum EventCountry {
    Jp,
    En,
    Kr,
    Tw,
}

impl EventCountry {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "jp" | "ja" => Some(Self::Jp),
            "en" => Some(Self::En),
            "kr" | "ko" => Some(Self::Kr),
            "tw" => Some(Self::Tw),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct SelectedRequest {
    data_type: EventDataType,
    country: EventCountry,
    file: bool,
    encrypted: bool,
    kbc: bool,
}

enum EventDataRequest {
    DefaultLinks,
    All {
        country: EventCountry,
        file: bool,
        encrypted: bool,
        kbc: bool,
    },
    Selected(SelectedRequest),
    Invalid,
}

pub(super) struct EventDataCommand {
    metadata: CommandMetadata,
    help: String,
    data_source: EventDataSource,
}

impl EventDataCommand {
    pub(super) fn new(help: &str, http: Arc<HttpService>, clock: Arc<dyn Clock>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "eventdata".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            data_source: EventDataSource::new(http, clock),
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
        let result = match parse_request(arguments) {
            EventDataRequest::Invalid => return Ok(message(context.channel_id(), INVALID_MESSAGE)),
            EventDataRequest::DefaultLinks => self
                .data_source
                .official_links(&DEFAULT_TYPES, EventCountry::Jp)
                .await
                .map(|links| links_output(context.channel_id(), &DEFAULT_TYPES, &links)),
            EventDataRequest::All {
                country,
                file,
                encrypted,
                kbc,
            } => {
                if file {
                    self.all_attachments(context.channel_id(), country, encrypted, kbc)
                        .await
                } else if kbc {
                    let links = ALL_TYPES
                        .iter()
                        .map(|data_type| {
                            (*data_type, build_kbc_url(*data_type, country, encrypted))
                        })
                        .collect();
                    Ok(links_output(context.channel_id(), &ALL_TYPES, &links))
                } else {
                    self.data_source
                        .official_links(&ALL_TYPES, country)
                        .await
                        .map(|links| links_output(context.channel_id(), &ALL_TYPES, &links))
                }
            }
            EventDataRequest::Selected(request) => {
                if request.file {
                    self.data_source
                        .attachment(request)
                        .await
                        .map(|attachment| attachment_output(context.channel_id(), attachment))
                } else if request.kbc {
                    let links = HashMap::from([(
                        request.data_type,
                        build_kbc_url(request.data_type, request.country, request.encrypted),
                    )]);
                    Ok(links_output(
                        context.channel_id(),
                        &[request.data_type],
                        &links,
                    ))
                } else {
                    self.data_source
                        .official_links(&[request.data_type], request.country)
                        .await
                        .map(|links| {
                            links_output(context.channel_id(), &[request.data_type], &links)
                        })
                }
            }
        };
        match result {
            Ok(output) => Ok(output),
            Err(error) => {
                eprintln!("Event data retrieval failed: {error}");
                Ok(message(context.channel_id(), DATA_ERROR_MESSAGE))
            }
        }
    }

    async fn all_attachments(
        &self,
        channel_id: &str,
        country: EventCountry,
        encrypted: bool,
        kbc: bool,
    ) -> Result<CommandOutput, data_source::EventDataError> {
        let mut tasks = JoinSet::new();
        for (index, data_type) in ALL_TYPES.into_iter().enumerate() {
            let data_source = self.data_source.clone();
            tasks.spawn(async move {
                let attachment = data_source
                    .attachment(SelectedRequest {
                        data_type,
                        country,
                        file: true,
                        encrypted,
                        kbc,
                    })
                    .await?;
                Ok::<_, data_source::EventDataError>((index, attachment))
            });
        }
        let mut attachments = Vec::with_capacity(ALL_TYPES.len());
        while let Some(result) = tasks.join_next().await {
            let value = result.map_err(|error| {
                data_source::EventDataError::new(format!("attachment task failed: {error}"))
            })??;
            attachments.push(value);
        }
        attachments.sort_by_key(|(index, _)| *index);
        let mut output = CommandOutput::new();
        for (_, attachment) in attachments {
            output
                .push(attachment_action(channel_id, attachment))
                .map_err(|error| data_source::EventDataError::new(error.to_string()))?;
        }
        Ok(output)
    }
}

impl Command for EventDataCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { self.run(&context, &arguments).await })
    }
}

fn parse_request(arguments: &[String]) -> EventDataRequest {
    if arguments.is_empty() {
        return EventDataRequest::DefaultLinks;
    }
    let all = arguments[0].eq_ignore_ascii_case("all");
    let data_type = if all {
        None
    } else {
        match EventDataType::parse(&arguments[0]) {
            Some(value) => Some(value),
            None => return EventDataRequest::Invalid,
        }
    };
    let mut country = EventCountry::Jp;
    let mut country_seen = false;
    let mut file = false;
    let mut encrypted = false;
    let mut kbc = false;
    for argument in &arguments[1..] {
        if let Some(value) = EventCountry::parse(argument) {
            if country_seen {
                return EventDataRequest::Invalid;
            }
            country = value;
            country_seen = true;
        } else if argument.eq_ignore_ascii_case("tsv") || argument.eq_ignore_ascii_case("file") {
            if file {
                return EventDataRequest::Invalid;
            }
            file = true;
        } else if argument.eq_ignore_ascii_case("enc") {
            if encrypted {
                return EventDataRequest::Invalid;
            }
            encrypted = true;
        } else if argument.eq_ignore_ascii_case("kbc") {
            if kbc {
                return EventDataRequest::Invalid;
            }
            kbc = true;
        } else {
            return EventDataRequest::Invalid;
        }
    }
    if encrypted && !file && !kbc {
        return EventDataRequest::Invalid;
    }
    match data_type {
        Some(data_type) => EventDataRequest::Selected(SelectedRequest {
            data_type,
            country,
            file,
            encrypted,
            kbc,
        }),
        None => EventDataRequest::All {
            country,
            file,
            encrypted,
            kbc,
        },
    }
}

fn links_output(
    channel_id: &str,
    types: &[EventDataType],
    links: &HashMap<EventDataType, String>,
) -> CommandOutput {
    let content = types
        .iter()
        .map(|data_type| {
            format!(
                "[{}]\n{}",
                data_type.label(),
                links.get(data_type).map(String::as_str).unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    message(channel_id, content)
}

fn attachment_output(channel_id: &str, attachment: EventAttachment) -> CommandOutput {
    CommandOutput::single(attachment_action(channel_id, attachment))
}

fn attachment_action(channel_id: &str, attachment: EventAttachment) -> CoreActionData {
    CoreActionData::SendAttachment {
        channel_id: channel_id.to_owned(),
        file_name: attachment.file_name,
        content_type: None,
        message: None,
        data: attachment.data,
    }
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}
