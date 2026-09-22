//! Asset file選択のページ表示と継続Sessionを共通化する。

use std::sync::Arc;

use kbc_protocol::CoreActionData;

use crate::session::{SessionContext, SessionContinuation, SessionFuture, SessionResume};

use super::remote_data::RemoteAssetSource;

pub(in crate::commands) const NUMBER_EMOJIS: [&str; 9] =
    ["1️⃣", "2️⃣", "3️⃣", "4️⃣", "5️⃣", "6️⃣", "7️⃣", "8️⃣", "9️⃣"];
pub(in crate::commands) const PREVIOUS_PAGE_EMOJI: &str = "◀️";
pub(in crate::commands) const NEXT_PAGE_EMOJI: &str = "▶️";
const PAGE_SIZE: usize = NUMBER_EMOJIS.len();

#[derive(Clone)]
pub(in crate::commands) struct AssetFileOption {
    pub(in crate::commands) relative_path: String,
    pub(in crate::commands) label: String,
}

pub(in crate::commands) fn file_picker(
    title: String,
    options: Vec<AssetFileOption>,
    assets: RemoteAssetSource,
    error_message: &'static str,
) -> (String, Box<dyn SessionContinuation>) {
    let continuation =
        FilePickerContinuation::new(title, Arc::new(options), 0, assets, error_message);
    let content = continuation.format("数字のリアクションでファイルを選択してください。");
    (content, Box::new(continuation))
}

struct FilePickerContinuation {
    title: String,
    options: Arc<Vec<AssetFileOption>>,
    page_index: usize,
    reactions: Vec<String>,
    assets: RemoteAssetSource,
    error_message: &'static str,
}

impl FilePickerContinuation {
    fn new(
        title: String,
        options: Arc<Vec<AssetFileOption>>,
        page_index: usize,
        assets: RemoteAssetSource,
        error_message: &'static str,
    ) -> Self {
        let page_count = options.len().div_ceil(PAGE_SIZE);
        let item_count = options
            .len()
            .saturating_sub(page_index * PAGE_SIZE)
            .min(PAGE_SIZE);
        let mut reactions = NUMBER_EMOJIS[..item_count]
            .iter()
            .map(|emoji| (*emoji).to_owned())
            .collect::<Vec<_>>();
        if page_index > 0 {
            reactions.push(PREVIOUS_PAGE_EMOJI.to_owned());
        }
        if page_index + 1 < page_count {
            reactions.push(NEXT_PAGE_EMOJI.to_owned());
        }
        Self {
            title,
            options,
            page_index,
            reactions,
            assets,
            error_message,
        }
    }

    fn format(&self, footer: &str) -> String {
        let page_count = self.options.len().div_ceil(PAGE_SIZE);
        let start_index = self.page_index * PAGE_SIZE;
        let page = &self.options[start_index..self.options.len().min(start_index + PAGE_SIZE)];
        let lines = page
            .iter()
            .enumerate()
            .map(|(index, option)| {
                format!(
                    "{} {} ({})",
                    NUMBER_EMOJIS[index], option.relative_path, option.label
                )
            })
            .collect::<Vec<_>>();
        format!(
            "{} {}～{}/{}（{}/{}ページ）\n```text\n{}\n```\n{}",
            self.title,
            start_index + 1,
            start_index + page.len(),
            self.options.len(),
            self.page_index + 1,
            page_count,
            lines.join("\n"),
            footer
        )
    }
}

impl SessionContinuation for FilePickerContinuation {
    fn reactions(&self) -> &[String] {
        &self.reactions
    }

    fn resume(self: Box<Self>, context: SessionContext, reaction: String) -> SessionFuture {
        Box::pin(async move {
            if reaction == PREVIOUS_PAGE_EMOJI || reaction == NEXT_PAGE_EMOJI {
                let page_index = if reaction == PREVIOUS_PAGE_EMOJI {
                    self.page_index.saturating_sub(1)
                } else {
                    self.page_index + 1
                };
                let next = FilePickerContinuation::new(
                    self.title.clone(),
                    Arc::clone(&self.options),
                    page_index,
                    self.assets.clone(),
                    self.error_message,
                );
                let content = next.format("数字のリアクションでファイルを選択してください。");
                return Ok(SessionResume::continue_with(
                    vec![CoreActionData::EditMessage {
                        channel_id: context.channel_id().to_owned(),
                        message_id: context.message_id().to_owned(),
                        content,
                    }],
                    Box::new(next),
                ));
            }

            let Some(index) = NUMBER_EMOJIS.iter().position(|emoji| *emoji == reaction) else {
                return Ok(SessionResume::complete(Vec::new()));
            };
            let option_index = self.page_index * PAGE_SIZE + index;
            let Some(selected) = self.options.get(option_index) else {
                return Ok(SessionResume::complete(Vec::new()));
            };
            let mut actions = vec![CoreActionData::EditMessage {
                channel_id: context.channel_id().to_owned(),
                message_id: context.message_id().to_owned(),
                content: self.format(&format!("選択済み: {}", selected.relative_path)),
            }];
            match self
                .assets
                .attachment(context.channel_id(), &selected.relative_path)
                .await
            {
                Ok(attachment) => actions.push(attachment),
                Err(error) => {
                    eprintln!("Asset file retrieval failed: {error}");
                    actions.push(CoreActionData::SendMessage {
                        channel_id: context.channel_id().to_owned(),
                        content: self.error_message.to_owned(),
                    });
                }
            }
            Ok(SessionResume::complete(actions))
        })
    }

    fn timeout_content(&self) -> Option<String> {
        Some(self.format("ファイル選択受付は終了しました。"))
    }
}
