//! ボタン・選択メニュー・Modalを使う統合設定CommandとInteraction処理。

use std::sync::Arc;

use kbc_protocol::{
    ActionOutcome, ButtonStyle, CoreActionData, CoreEvent, CoreEventData, InteractionField,
    MessageComponentData, MessageComponentRow, ModalComponentData, ModalDefinition,
    ModalTextInputStyle, RequestId, SelectOption,
};

use crate::command::{Command, CommandContext, CommandFuture, CommandMetadata, CommandOutput};
use crate::notification_role_settings::NotificationRoleSettingsService;
use crate::permissions::{
    has_maintainer_access, is_bot_administrator, try_has_maintainer_identity_access,
};
use crate::storage::{
    MAX_MAINTAINER_SUBJECTS, MAX_NOTIFICATION_ROLES, MAX_SKD_RELATED_URLS, NotificationCategory,
    StorageService, Subscription,
};
use crate::task_runtime::TaskRuntime;

const SETTINGS_PREFIX: &str = "settings:";
const NOTIFICATION_PAGE_SIZE: usize = 5;
const MAINTAINER_PAGE_SIZE: usize = 20;

pub(super) struct SettingsCommand {
    metadata: CommandMetadata,
    help: String,
    storage: Arc<StorageService>,
}

impl SettingsCommand {
    pub(super) fn new(help: &str, storage: Arc<StorageService>) -> Self {
        Self {
            metadata: CommandMetadata {
                name: "settings".to_owned(),
                aliases: Vec::new(),
                guild_only: true,
            },
            help: help.to_owned(),
            storage,
        }
    }

    async fn run(&self, context: &CommandContext, arguments: &[String]) -> CommandOutput {
        if arguments.len() == 1 && arguments[0].eq_ignore_ascii_case("help") {
            return message(context.channel_id(), &self.help);
        }
        if !arguments.is_empty() {
            return message(context.channel_id(), "使い方: o.settings");
        }
        match has_maintainer_access(&self.storage, context).await {
            Ok(true) => CommandOutput::single(CoreActionData::SendInteractiveMessage {
                channel_id: context.channel_id().to_owned(),
                content: "設定を開くには、下のボタンを押してください。".to_owned(),
                rows: vec![row(vec![button(
                    format!("settings:open:{}", context.user_id()),
                    "設定を開く",
                    ButtonStyle::Primary,
                )])],
            }),
            Ok(false) => message(
                context.channel_id(),
                "この操作にはBot管理者・Botメンテナー権限が必要です。",
            ),
            Err(error) => {
                eprintln!("Settings permission load failed: {error}");
                message(
                    context.channel_id(),
                    "❌ 権限情報の読み込みに失敗しました。時間をおいて再度お試しください。",
                )
            }
        }
    }
}

impl Command for SettingsCommand {
    fn metadata(&self) -> &CommandMetadata {
        &self.metadata
    }

    fn execute<'a>(&'a self, context: CommandContext, arguments: Vec<String>) -> CommandFuture<'a> {
        Box::pin(async move { Ok(self.run(&context, &arguments).await) })
    }
}

#[derive(Clone)]
pub(crate) struct SettingsInteractionService {
    storage: Arc<StorageService>,
    tasks: Arc<TaskRuntime>,
    role_settings: Arc<NotificationRoleSettingsService>,
}

impl SettingsInteractionService {
    pub(crate) fn new(
        storage: Arc<StorageService>,
        tasks: Arc<TaskRuntime>,
        role_settings: Arc<NotificationRoleSettingsService>,
    ) -> Self {
        Self {
            storage,
            tasks,
            role_settings,
        }
    }

    pub(crate) fn handles_event(event: &CoreEvent) -> bool {
        matches!(
            &event.event,
            CoreEventData::ComponentInteraction { custom_id, .. }
                | CoreEventData::ModalSubmit { custom_id, .. }
                if custom_id.starts_with(SETTINGS_PREFIX)
        )
    }

    pub(crate) fn busy_action(event: &CoreEvent) -> Option<CoreActionData> {
        let context = InteractionContext::from_event(event)?;
        context.custom_id.starts_with(SETTINGS_PREFIX).then(|| {
            context.error_action("設定処理が混み合っています。少し待ってから再度お試しください。")
        })
    }

    pub(crate) async fn handle_event(&self, event: &CoreEvent) -> Option<Vec<CoreActionData>> {
        let context = InteractionContext::from_event(event)?;
        if !context.custom_id.starts_with(SETTINGS_PREFIX) {
            return None;
        }
        let Some(guild_id) = context.guild_id else {
            return Some(vec![
                context.error_action("設定画面はサーバー内でのみ使用できます。"),
            ]);
        };
        match try_has_maintainer_identity_access(
            &self.storage,
            context.user_id,
            context.member_role_ids,
        ) {
            Ok(true) => {}
            Ok(false) => {
                return Some(vec![context.error_action(
                    "この操作にはBot管理者・Botメンテナー権限が必要です。",
                )]);
            }
            Err(error) => {
                let content = if matches!(error.code(), "storage-busy" | "storage-not-ready") {
                    "別の設定を保存中です。少し待ってから再度お試しください。"
                } else {
                    eprintln!("Settings interaction permission load failed: {error}");
                    "❌ 権限情報の読み込みに失敗しました。"
                };
                return Some(vec![context.error_action(content)]);
            }
        }
        let action = if context.is_modal {
            self.handle_modal_submit(&context, guild_id).await
        } else {
            self.handle_component(&context, guild_id).await
        };
        Some(vec![action])
    }

    async fn handle_component(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
    ) -> CoreActionData {
        if let Some(owner_id) = context.custom_id.strip_prefix("settings:open:") {
            if owner_id != context.user_id {
                return context.reply("この設定画面はコマンド実行者専用です。", true, Vec::new());
            }
            return context.reply("設定メニュー", true, home_rows());
        }
        match context.custom_id {
            "settings:home" => context.update("設定メニュー", home_rows()),
            "settings:notifications" => context.update(
                "通知設定\n編集する通知種別を選択してください。",
                notification_rows(),
            ),
            "settings:notification-category" => {
                let Some(category) = context.single_value().and_then(parse_notification_category)
                else {
                    return context.reply("通知種別を確認できませんでした。", true, Vec::new());
                };
                self.notification_page(context, guild_id, category, 0).await
            }
            "settings:roles" => self.role_page(context, guild_id).await,
            "settings:role-add" => {
                let Some(role_id) = context
                    .single_value()
                    .filter(|value| valid_snowflake(value))
                else {
                    return context.reply("ロールを確認できませんでした。", true, Vec::new());
                };
                context.show_modal(confirm_modal(
                    format!("settings:modal:role-add:{role_id}"),
                    "通知ロールを登録",
                    format!("<@&{role_id}> を通知ロールへ登録します。"),
                    "登録する",
                ))
            }
            "settings:role-remove" => {
                let Some(role_id) = context
                    .single_value()
                    .filter(|value| valid_snowflake(value))
                else {
                    return context.reply("ロールを確認できませんでした。", true, Vec::new());
                };
                context.show_modal(confirm_modal(
                    format!("settings:modal:role-remove:{role_id}"),
                    "通知ロールを削除",
                    format!("<@&{role_id}> を通知ロールから削除します。関連する通知メンションも解除されます。"),
                    "削除する",
                ))
            }
            "settings:role-create" => context.show_modal(role_create_modal()),
            "settings:panel-current" => context.show_modal(confirm_modal(
                format!("settings:modal:panel:{}", context.channel_id),
                "ロール選択パネルを設置",
                format!("<#{}> にロール選択パネルを設置します。", context.channel_id),
                "設置する",
            )),
            "settings:panel-channel" => {
                let Some(channel_id) = context
                    .single_value()
                    .filter(|value| valid_snowflake(value))
                else {
                    return context.reply("チャンネルを確認できませんでした。", true, Vec::new());
                };
                context.show_modal(confirm_modal(
                    format!("settings:modal:panel:{channel_id}"),
                    "ロール選択パネルを設置",
                    format!("<#{channel_id}> にロール選択パネルを設置します。"),
                    "設置する",
                ))
            }
            "settings:maintainers" => self.maintainer_page(context, guild_id, 0).await,
            "settings:maintainer-add-user" => self.maintainer_add_modal(context, "user"),
            "settings:maintainer-add-role" => self.maintainer_add_modal(context, "role"),
            "settings:maintainer-remove" => {
                let Some(subject_id) = context
                    .single_value()
                    .filter(|value| valid_subject_id(value))
                else {
                    return context.reply("削除対象を確認できませんでした。", true, Vec::new());
                };
                context.show_modal(confirm_modal(
                    format!("settings:modal:maintainer-remove:{subject_id}"),
                    "Botメンテナーを削除",
                    format!("登録対象 `{subject_id}` をBotメンテナーから削除します。"),
                    "削除する",
                ))
            }
            _ => {
                if let Some((category, page)) = parse_category_page(context.custom_id) {
                    return self
                        .notification_page(context, guild_id, category, page)
                        .await;
                }
                if let Some(category) =
                    parse_category_action(context.custom_id, "settings:notification-current:")
                {
                    return self
                        .notification_modal_action(context, guild_id, category, context.channel_id)
                        .await;
                }
                if let Some(category) =
                    parse_category_action(context.custom_id, "settings:notification-channel:")
                {
                    let Some(channel_id) = context
                        .single_value()
                        .filter(|value| valid_snowflake(value))
                    else {
                        return context.reply(
                            "チャンネルを確認できませんでした。",
                            true,
                            Vec::new(),
                        );
                    };
                    return self
                        .notification_modal_action(context, guild_id, category, channel_id)
                        .await;
                }
                if let Some(category) =
                    parse_category_action(context.custom_id, "settings:notification-urls:")
                    && category == NotificationCategory::Skd
                {
                    return self.related_urls_modal_action(context, guild_id).await;
                }
                if let Some(page) = parse_page(context.custom_id, "settings:maintainer-page:") {
                    return self.maintainer_page(context, guild_id, page).await;
                }
                context.reply("設定操作を確認できませんでした。", true, Vec::new())
            }
        }
    }

    async fn handle_modal_submit(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
    ) -> CoreActionData {
        if let Some(target) = context
            .custom_id
            .strip_prefix("settings:modal:notification:")
        {
            let mut parts = target.split(':');
            let Some(category) = parts.next().and_then(parse_notification_category) else {
                return context.edit_reply("通知種別を確認できませんでした。", Vec::new());
            };
            let Some(channel_id) = parts.next().filter(|value| valid_snowflake(value)) else {
                return context.edit_reply("通知チャンネルを確認できませんでした。", Vec::new());
            };
            if parts.next().is_some() {
                return context.edit_reply("通知設定を確認できませんでした。", Vec::new());
            }
            return self
                .save_notification(context, guild_id, category, channel_id)
                .await;
        }
        if context.custom_id == "settings:modal:urls" {
            return self.save_related_urls(context, guild_id).await;
        }
        if let Some(role_id) = context
            .custom_id
            .strip_prefix("settings:modal:role-add:")
            .filter(|value| valid_snowflake(value))
        {
            return self.save_role(context, guild_id, role_id, true).await;
        }
        if let Some(role_id) = context
            .custom_id
            .strip_prefix("settings:modal:role-remove:")
            .filter(|value| valid_snowflake(value))
        {
            return self.save_role(context, guild_id, role_id, false).await;
        }
        if context.custom_id == "settings:modal:role-create" {
            return self.create_role(context, guild_id).await;
        }
        if let Some(channel_id) = context
            .custom_id
            .strip_prefix("settings:modal:panel:")
            .filter(|value| valid_snowflake(value))
        {
            return self.publish_panel(context, guild_id, channel_id).await;
        }
        if let Some(target) = context
            .custom_id
            .strip_prefix("settings:modal:maintainer-add:")
        {
            let mut parts = target.split(':');
            let kind = parts.next();
            let Some(subject_id) = parts.next().filter(|value| valid_subject_id(value)) else {
                return context.edit_reply("追加対象を確認できませんでした。", Vec::new());
            };
            if !matches!(kind, Some("user" | "role")) || parts.next().is_some() {
                return context.edit_reply("追加対象を確認できませんでした。", Vec::new());
            }
            return self.save_maintainer(context, subject_id, true).await;
        }
        if let Some(subject_id) = context
            .custom_id
            .strip_prefix("settings:modal:maintainer-remove:")
            .filter(|value| valid_subject_id(value))
        {
            return self.save_maintainer(context, subject_id, false).await;
        }
        context.edit_reply("設定操作を確認できませんでした。", Vec::new())
    }

    async fn notification_page(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
        category: NotificationCategory,
        requested_page: usize,
    ) -> CoreActionData {
        let subscriptions = match self.storage.guild_subscriptions(guild_id).await {
            Ok(subscriptions) => subscriptions
                .into_iter()
                .filter(|subscription| subscription.category == category)
                .collect::<Vec<_>>(),
            Err(error) => {
                eprintln!("Notification settings load failed: {error}");
                return context.reply("❌ 通知設定を読み込めませんでした。", true, Vec::new());
            }
        };
        let page_count = subscriptions.len().div_ceil(NOTIFICATION_PAGE_SIZE).max(1);
        let page = requested_page.min(page_count - 1);
        let start = page * NOTIFICATION_PAGE_SIZE;
        let mut lines = vec![notification_label(category).to_owned()];
        if subscriptions.is_empty() {
            lines.push("登録済みの通知チャンネルはありません。".to_owned());
        } else {
            lines.push(format!(
                "登録済み通知チャンネル（{}/{page_count}ページ）",
                page + 1
            ));
            lines.extend(
                subscriptions[start..subscriptions.len().min(start + NOTIFICATION_PAGE_SIZE)]
                    .iter()
                    .map(|subscription| {
                        let roles = if subscription.role_ids.is_empty() {
                            "メンションなし".to_owned()
                        } else {
                            subscription
                                .role_ids
                                .iter()
                                .map(|role_id| format!("<@&{role_id}>"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        };
                        format!("- <#{}> — {roles}", subscription.channel_id)
                    }),
            );
        }
        lines.push("設定するチャンネルを下から選択してください。".to_owned());
        let key = notification_key(category);
        let mut navigation = Vec::new();
        if page > 0 {
            navigation.push(button(
                format!("settings:notification-page:{key}:{}", page - 1),
                "前へ",
                ButtonStyle::Secondary,
            ));
        }
        if page + 1 < page_count {
            navigation.push(button(
                format!("settings:notification-page:{key}:{}", page + 1),
                "次へ",
                ButtonStyle::Secondary,
            ));
        }
        navigation.push(button(
            format!("settings:notification-current:{key}"),
            "このチャンネルを設定",
            ButtonStyle::Primary,
        ));
        if category == NotificationCategory::Skd {
            navigation.push(button(
                format!("settings:notification-urls:{key}"),
                "関連サイトURL",
                ButtonStyle::Secondary,
            ));
        }
        let rows = vec![
            row(vec![MessageComponentData::ChannelSelect {
                custom_id: format!("settings:notification-channel:{key}"),
                placeholder: "別の通知チャンネルを選択".to_owned(),
                min_values: 1,
                max_values: 1,
            }]),
            row(navigation),
            row(vec![button(
                "settings:notifications",
                "通知種別へ戻る",
                ButtonStyle::Secondary,
            )]),
        ];
        context.update(lines.join("\n"), rows)
    }

    async fn notification_modal_action(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
        category: NotificationCategory,
        channel_id: &str,
    ) -> CoreActionData {
        let (subscriptions, role_settings) =
            match self.storage.try_notification_settings_snapshot(guild_id) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    if !matches!(error.code(), "storage-busy" | "storage-not-ready") {
                        eprintln!("Notification editor snapshot failed: {error}");
                    }
                    return context.reply(
                        "別の設定を保存中です。少し待ってから再度お試しください。",
                        true,
                        Vec::new(),
                    );
                }
            };
        let current = subscriptions.iter().find(|subscription| {
            subscription.category == category && subscription.channel_id == channel_id
        });
        let mut components = vec![
            ModalComponentData::TextDisplay {
                content: format!(
                    "**{}**\n通知先: <#{channel_id}>\n現在: {}",
                    notification_label(category),
                    if current.is_some() {
                        "登録済み"
                    } else {
                        "未登録"
                    },
                ),
            },
            ModalComponentData::StringSelect {
                custom_id: "operation".to_owned(),
                label: "操作".to_owned(),
                description: None,
                options: vec![
                    option("設定を保存", "save", None, true),
                    option("通知先を解除", "remove", None, false),
                ],
                min_values: 1,
                max_values: 1,
            },
        ];
        if !role_settings.roles.is_empty() {
            components.push(ModalComponentData::StringSelect {
                custom_id: "roles".to_owned(),
                label: "メンションする通知ロール".to_owned(),
                description: Some("未選択ならロールをメンションしません".to_owned()),
                options: role_settings
                    .roles
                    .iter()
                    .map(|role| {
                        option(
                            &role.name,
                            &role.role_id,
                            Some(&role.role_id),
                            current.is_some_and(|subscription| {
                                subscription.role_ids.contains(&role.role_id)
                            }),
                        )
                    })
                    .collect(),
                min_values: 0,
                max_values: role_settings.roles.len() as u8,
            });
        }
        context.show_modal(ModalDefinition {
            custom_id: format!(
                "settings:modal:notification:{}:{channel_id}",
                notification_key(category)
            ),
            title: format!("{}設定", notification_label(category)),
            components,
        })
    }

    async fn save_notification(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
        category: NotificationCategory,
        channel_id: &str,
    ) -> CoreActionData {
        let operation = context.field_first("operation");
        let enabled = match operation {
            Some("save") => true,
            Some("remove") => false,
            _ => return context.edit_reply("操作を確認できませんでした。", Vec::new()),
        };
        let role_ids = if enabled {
            context.field_values("roles").unwrap_or_default().to_vec()
        } else {
            Vec::new()
        };
        if enabled {
            match self
                .tasks
                .request_adapter(
                    context.request_id,
                    CoreActionData::ResolveSendableChannel {
                        guild_id: guild_id.to_owned(),
                        channel_id: channel_id.to_owned(),
                    },
                )
                .await
            {
                Ok(ActionOutcome::ChannelResolved { .. }) => {}
                Ok(ActionOutcome::Failure { code, .. }) => {
                    eprintln!("Notification channel resolution failed: {code}");
                    return context.edit_reply(
                        "❌ Botが送信できるチャンネルを選択してください。",
                        notification_back_rows(category),
                    );
                }
                Ok(outcome) => {
                    eprintln!("Unexpected channel resolution outcome: {outcome:?}");
                    return context.edit_reply(
                        "❌ 通知チャンネルを確認できませんでした。",
                        notification_back_rows(category),
                    );
                }
                Err(error) => {
                    eprintln!("Notification channel resolution failed: {error}");
                    return context.edit_reply(
                        "❌ 通知チャンネルを確認できませんでした。",
                        notification_back_rows(category),
                    );
                }
            }
        }
        let subscription = Subscription {
            guild_id: guild_id.to_owned(),
            channel_id: channel_id.to_owned(),
            category,
            role_ids,
        };
        match self
            .storage
            .set_subscription_config(subscription, enabled)
            .await
        {
            Ok(changed) => context.edit_reply(
                match (enabled, changed) {
                    (true, true) => "通知設定を保存しました。",
                    (true, false) => "通知設定に変更はありません。",
                    (false, true) => "通知先を解除しました。",
                    (false, false) => "このチャンネルは通知先に登録されていません。",
                },
                notification_back_rows(category),
            ),
            Err(error) => {
                eprintln!("Notification settings save failed: {error}");
                context.edit_reply(
                    "❌ 通知設定の保存に失敗しました。",
                    notification_back_rows(category),
                )
            }
        }
    }

    async fn related_urls_modal_action(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
    ) -> CoreActionData {
        match self.storage.try_skd_related_urls(guild_id) {
            Ok(urls) => context.show_modal(ModalDefinition {
                custom_id: "settings:modal:urls".to_owned(),
                title: "スケジュール関連サイトURL".to_owned(),
                components: vec![
                    ModalComponentData::TextDisplay {
                        content: format!(
                            "1行に1件入力してください。最大{MAX_SKD_RELATED_URLS}件です。空欄で全件削除します。"
                        ),
                    },
                    ModalComponentData::TextInput {
                        custom_id: "urls".to_owned(),
                        label: "関連サイトURL".to_owned(),
                        description: None,
                        style: ModalTextInputStyle::Paragraph,
                        required: false,
                        value: Some(urls.join("\n")),
                        placeholder: Some("https://example.com/".to_owned()),
                        max_length: Some(2_000),
                    },
                ],
            }),
            Err(error) => {
                if !matches!(error.code(), "storage-busy" | "storage-not-ready") {
                    eprintln!("Related URL settings load failed: {error}");
                }
                context.reply(
                    "別の設定を保存中です。少し待ってから再度お試しください。",
                    true,
                    Vec::new(),
                )
            }
        }
    }

    async fn save_related_urls(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
    ) -> CoreActionData {
        let urls = context
            .field_first("urls")
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if urls.len() > MAX_SKD_RELATED_URLS {
            return context.edit_reply(
                format!("関連サイトURLは最大{MAX_SKD_RELATED_URLS}件です。"),
                notification_back_rows(NotificationCategory::Skd),
            );
        }
        match self.storage.set_skd_related_urls(guild_id, urls).await {
            Ok(changed) => context.edit_reply(
                if changed {
                    "関連サイトURLを保存しました。"
                } else {
                    "関連サイトURLに変更はありません。"
                },
                notification_back_rows(NotificationCategory::Skd),
            ),
            Err(error) => {
                let content = match error.code() {
                    "invalid-related-url" => {
                        "❌ `http://` または `https://` で始まるURLを1行ずつ入力してください。"
                    }
                    "related-url-limit-reached" => {
                        "❌ URLの重複、件数、または合計文字数を確認してください。"
                    }
                    _ => {
                        eprintln!("Related URL settings save failed: {error}");
                        "❌ 関連サイトURLの保存に失敗しました。"
                    }
                };
                context.edit_reply(content, notification_back_rows(NotificationCategory::Skd))
            }
        }
    }

    async fn role_page(&self, context: &InteractionContext<'_>, guild_id: &str) -> CoreActionData {
        let settings = match self.storage.notification_role_settings(guild_id).await {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!("Notification role settings load failed: {error}");
                return context.reply(
                    "❌ 通知ロール設定を読み込めませんでした。",
                    true,
                    Vec::new(),
                );
            }
        };
        let mut lines = vec![format!(
            "通知ロール共通設定（最大{MAX_NOTIFICATION_ROLES}件）"
        )];
        if settings.roles.is_empty() {
            lines.push("登録済みの通知ロールはありません。".to_owned());
        } else {
            lines.extend(
                settings
                    .roles
                    .iter()
                    .map(|role| format!("- <@&{}> — {}", role.role_id, role.name)),
            );
        }
        lines.push(match settings.panel {
            Some(panel) => format!("選択パネル: <#{}>", panel.channel_id),
            None => "選択パネル: 未設置".to_owned(),
        });
        let mut rows = vec![row(vec![MessageComponentData::RoleSelect {
            custom_id: "settings:role-add".to_owned(),
            placeholder: "既存ロールを通知ロールへ登録".to_owned(),
            min_values: 1,
            max_values: 1,
        }])];
        if !settings.roles.is_empty() {
            rows.push(row(vec![MessageComponentData::StringSelect {
                custom_id: "settings:role-remove".to_owned(),
                placeholder: "登録済み通知ロールを削除".to_owned(),
                options: settings
                    .roles
                    .iter()
                    .map(|role| option(&role.name, &role.role_id, Some(&role.role_id), false))
                    .collect(),
                min_values: 1,
                max_values: 1,
            }]));
        }
        let mut role_actions = Vec::new();
        if settings.roles.len() < MAX_NOTIFICATION_ROLES {
            role_actions.push(button(
                "settings:role-create",
                "通知ロールを新規作成",
                ButtonStyle::Primary,
            ));
        }
        role_actions.push(button(
            "settings:panel-current",
            "このチャンネルにパネル設置",
            ButtonStyle::Secondary,
        ));
        rows.push(row(role_actions));
        rows.push(row(vec![MessageComponentData::ChannelSelect {
            custom_id: "settings:panel-channel".to_owned(),
            placeholder: "別のチャンネルにパネルを設置".to_owned(),
            min_values: 1,
            max_values: 1,
        }]));
        rows.push(row(vec![button(
            "settings:home",
            "設定メニューへ戻る",
            ButtonStyle::Secondary,
        )]));
        context.update(lines.join("\n"), rows)
    }

    async fn save_role(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
        role_id: &str,
        enabled: bool,
    ) -> CoreActionData {
        if !context.confirmed() {
            return context.edit_reply("操作を確認できませんでした。", role_back_rows());
        }
        match self
            .role_settings
            .set_role(context.request_id, guild_id, role_id, enabled)
            .await
        {
            Ok(result) => {
                let status = match (enabled, result.changed) {
                    (true, true) => "通知ロールへ登録しました。",
                    (true, false) => "このロールは登録済みです。",
                    (false, true) => {
                        "通知ロールから削除しました。関連する通知メンションも解除しました。"
                    }
                    (false, false) => "このロールは登録されていません。",
                };
                let content = if result.panel_updated {
                    status.to_owned()
                } else {
                    format!("{status}\n⚠️ 既存のロール選択パネルを更新できませんでした。")
                };
                context.edit_reply(content, role_back_rows())
            }
            Err(error) => context.edit_reply(role_error_message(error.code()), role_back_rows()),
        }
    }

    async fn create_role(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
    ) -> CoreActionData {
        let Some(name) = context.field_first("name").map(str::trim) else {
            return context.edit_reply("ロール名を入力してください。", role_back_rows());
        };
        if name.is_empty() || name.chars().count() > 100 {
            return context
                .edit_reply("ロール名は1〜100文字で入力してください。", role_back_rows());
        }
        match self.storage.notification_role_settings(guild_id).await {
            Ok(settings) if settings.roles.len() >= MAX_NOTIFICATION_ROLES => {
                return context.edit_reply(
                    format!("通知ロールは最大{MAX_NOTIFICATION_ROLES}件です。"),
                    role_back_rows(),
                );
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("Notification role settings load failed: {error}");
                return context.edit_reply(
                    "❌ 通知ロール設定を読み込めませんでした。",
                    role_back_rows(),
                );
            }
        }
        let role = match self
            .role_settings
            .create_role(context.request_id, guild_id, name.to_owned())
            .await
        {
            Ok(role) => role,
            Err(error) => {
                eprintln!("Settings role creation failed: {error}");
                return context.edit_reply("❌ ロールを作成できませんでした。", role_back_rows());
            }
        };
        match self
            .role_settings
            .set_role(context.request_id, guild_id, &role.role_id, true)
            .await
        {
            Ok(result) => {
                let suffix = if result.panel_updated {
                    ""
                } else {
                    "\n⚠️ 既存のロール選択パネルを更新できませんでした。"
                };
                context.edit_reply(
                    format!(
                        "通知ロール `@{}` (`{}`) を作成し、通知設定へ登録しました。{suffix}",
                        role.name, role.role_id
                    ),
                    role_back_rows(),
                )
            }
            Err(error) => {
                eprintln!("Created role registration failed: {error}");
                context.edit_reply(
                    format!(
                        "⚠️ ロール `@{}` (`{}`) は作成しましたが、通知設定への登録に失敗しました。",
                        role.name, role.role_id
                    ),
                    role_back_rows(),
                )
            }
        }
    }

    async fn publish_panel(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
        channel_id: &str,
    ) -> CoreActionData {
        if !context.confirmed() {
            return context.edit_reply("操作を確認できませんでした。", role_back_rows());
        }
        match self
            .tasks
            .request_adapter(
                context.request_id,
                CoreActionData::ResolveSendableChannel {
                    guild_id: guild_id.to_owned(),
                    channel_id: channel_id.to_owned(),
                },
            )
            .await
        {
            Ok(ActionOutcome::ChannelResolved { .. }) => {}
            Ok(outcome) => {
                eprintln!("Panel channel resolution failed: {outcome:?}");
                return context.edit_reply(
                    "❌ Botが送信できるチャンネルを選択してください。",
                    role_back_rows(),
                );
            }
            Err(error) => {
                eprintln!("Panel channel resolution failed: {error}");
                return context.edit_reply(
                    "❌ パネルの設置先を確認できませんでした。",
                    role_back_rows(),
                );
            }
        }
        match self
            .role_settings
            .publish_panel(context.request_id, guild_id, channel_id)
            .await
        {
            Ok(result) => context.edit_reply(
                if result.old_panel_disabled {
                    "ロール選択パネルを設置しました。"
                } else {
                    "ロール選択パネルを設置しました。旧パネルの無効化だけ失敗しました。"
                },
                role_back_rows(),
            ),
            Err(error) => {
                eprintln!("Settings panel publish failed: {error}");
                context.edit_reply(
                    "❌ ロール選択パネルを設置できませんでした。",
                    role_back_rows(),
                )
            }
        }
    }

    async fn maintainer_page(
        &self,
        context: &InteractionContext<'_>,
        guild_id: &str,
        requested_page: usize,
    ) -> CoreActionData {
        let subject_ids = match self.storage.maintainer_subject_ids().await {
            Ok(subject_ids) => subject_ids,
            Err(error) => {
                eprintln!("Maintainer settings load failed: {error}");
                return context.reply(
                    "❌ Botメンテナー設定を読み込めませんでした。",
                    true,
                    Vec::new(),
                );
            }
        };
        if !is_bot_administrator(context.user_id) {
            let members = self
                .tasks
                .request_adapter(
                    context.request_id,
                    CoreActionData::ResolveGuildMembers {
                        guild_id: guild_id.to_owned(),
                        subject_ids,
                        max_members: 25,
                    },
                )
                .await;
            let content = match members {
                Ok(ActionOutcome::MembersResolved { members, truncated }) => {
                    let mut lines = vec!["このサーバーのBotメンテナー".to_owned()];
                    if members.is_empty() {
                        lines.push("該当するメンテナーはいません。".to_owned());
                    } else {
                        lines.extend(members.into_iter().map(|member| {
                            format!("- {} (`{}`)", member.display_name, member.user_id)
                        }));
                        if truncated {
                            lines.push("- ほか（最大25人まで表示）".to_owned());
                        }
                    }
                    lines.push("追加・削除は固定Bot管理者だけが実行できます。".to_owned());
                    lines.join("\n")
                }
                Ok(outcome) => {
                    eprintln!("Unexpected maintainer resolution outcome: {outcome:?}");
                    "❌ Botメンテナー一覧を取得できませんでした。".to_owned()
                }
                Err(error) => {
                    eprintln!("Maintainer resolution failed: {error}");
                    "❌ Botメンテナー一覧を取得できませんでした。".to_owned()
                }
            };
            return context.update(
                content,
                vec![row(vec![button(
                    "settings:home",
                    "設定メニューへ戻る",
                    ButtonStyle::Secondary,
                )])],
            );
        }
        let page_count = subject_ids.len().div_ceil(MAINTAINER_PAGE_SIZE).max(1);
        let page = requested_page.min(page_count - 1);
        let start = page * MAINTAINER_PAGE_SIZE;
        let visible = &subject_ids[start..subject_ids.len().min(start + MAINTAINER_PAGE_SIZE)];
        let mut lines = vec![format!(
            "Botメンテナー設定（全サーバー共通・最大{MAX_MAINTAINER_SUBJECTS}件）"
        )];
        if visible.is_empty() {
            lines.push("登録対象はありません。".to_owned());
        } else {
            lines.push(format!("登録対象（{}/{page_count}ページ）", page + 1));
            lines.extend(visible.iter().map(|subject| format!("- `{subject}`")));
        }
        let mut rows = vec![
            row(vec![MessageComponentData::UserSelect {
                custom_id: "settings:maintainer-add-user".to_owned(),
                placeholder: "ユーザーをBotメンテナーへ追加".to_owned(),
                min_values: 1,
                max_values: 1,
            }]),
            row(vec![MessageComponentData::RoleSelect {
                custom_id: "settings:maintainer-add-role".to_owned(),
                placeholder: "ロールをBotメンテナーへ追加".to_owned(),
                min_values: 1,
                max_values: 1,
            }]),
        ];
        if !visible.is_empty() {
            rows.push(row(vec![MessageComponentData::StringSelect {
                custom_id: "settings:maintainer-remove".to_owned(),
                placeholder: "登録済み対象を削除".to_owned(),
                options: visible
                    .iter()
                    .enumerate()
                    .map(|(index, subject)| {
                        option(
                            &format!("登録対象 {}", start + index + 1),
                            subject,
                            Some(subject),
                            false,
                        )
                    })
                    .collect(),
                min_values: 1,
                max_values: 1,
            }]));
        }
        let mut navigation = Vec::new();
        if page > 0 {
            navigation.push(button(
                format!("settings:maintainer-page:{}", page - 1),
                "前へ",
                ButtonStyle::Secondary,
            ));
        }
        if page + 1 < page_count {
            navigation.push(button(
                format!("settings:maintainer-page:{}", page + 1),
                "次へ",
                ButtonStyle::Secondary,
            ));
        }
        navigation.push(button(
            "settings:home",
            "設定メニューへ戻る",
            ButtonStyle::Secondary,
        ));
        rows.push(row(navigation));
        context.update(lines.join("\n"), rows)
    }

    fn maintainer_add_modal(&self, context: &InteractionContext<'_>, kind: &str) -> CoreActionData {
        if !is_bot_administrator(context.user_id) {
            return context.reply(
                "Botメンテナーの変更は固定Bot管理者だけが実行できます。",
                true,
                Vec::new(),
            );
        }
        let Some(subject_id) = context
            .single_value()
            .filter(|value| valid_subject_id(value))
        else {
            return context.reply("追加対象を確認できませんでした。", true, Vec::new());
        };
        let mention = if kind == "role" {
            format!("<@&{subject_id}>")
        } else {
            format!("<@{subject_id}>")
        };
        context.show_modal(confirm_modal(
            format!("settings:modal:maintainer-add:{kind}:{subject_id}"),
            "Botメンテナーを追加",
            format!("{mention} を全サーバー共通のBotメンテナーへ追加します。"),
            "追加する",
        ))
    }

    async fn save_maintainer(
        &self,
        context: &InteractionContext<'_>,
        subject_id: &str,
        enabled: bool,
    ) -> CoreActionData {
        if !context.confirmed() {
            return context.edit_reply("操作を確認できませんでした。", maintainer_back_rows());
        }
        if !is_bot_administrator(context.user_id) {
            return context.edit_reply(
                "Botメンテナーの変更は固定Bot管理者だけが実行できます。",
                maintainer_back_rows(),
            );
        }
        match self.storage.set_maintainer(subject_id, enabled).await {
            Ok(changed) => context.edit_reply(
                match (enabled, changed) {
                    (true, true) => "Botメンテナー対象を追加しました。",
                    (true, false) => "この対象は追加済みです。",
                    (false, true) => "Botメンテナー対象を削除しました。",
                    (false, false) => "この対象は登録されていません。",
                },
                maintainer_back_rows(),
            ),
            Err(error) => {
                let content = if error.code() == "maintainer-limit-reached" {
                    format!("Botメンテナー対象は最大{MAX_MAINTAINER_SUBJECTS}件です。")
                } else {
                    eprintln!("Maintainer settings save failed: {error}");
                    "❌ Botメンテナー設定の保存に失敗しました。".to_owned()
                };
                context.edit_reply(content, maintainer_back_rows())
            }
        }
    }
}

struct InteractionContext<'a> {
    interaction_id: &'a str,
    guild_id: Option<&'a str>,
    channel_id: &'a str,
    user_id: &'a str,
    member_role_ids: &'a [String],
    custom_id: &'a str,
    values: &'a [String],
    fields: &'a [InteractionField],
    request_id: &'a RequestId,
    is_modal: bool,
}

impl<'a> InteractionContext<'a> {
    fn from_event(event: &'a CoreEvent) -> Option<Self> {
        match &event.event {
            CoreEventData::ComponentInteraction {
                interaction_id,
                guild_id,
                channel_id,
                user_id,
                member_role_ids,
                custom_id,
                values,
            } => Some(Self {
                interaction_id,
                guild_id: guild_id.as_deref(),
                channel_id,
                user_id,
                member_role_ids,
                custom_id,
                values,
                fields: &[],
                request_id: &event.request_id,
                is_modal: false,
            }),
            CoreEventData::ModalSubmit {
                interaction_id,
                guild_id,
                channel_id,
                user_id,
                member_role_ids,
                custom_id,
                fields,
            } => Some(Self {
                interaction_id,
                guild_id: guild_id.as_deref(),
                channel_id,
                user_id,
                member_role_ids,
                custom_id,
                values: &[],
                fields,
                request_id: &event.request_id,
                is_modal: true,
            }),
            _ => None,
        }
    }

    fn single_value(&self) -> Option<&str> {
        (self.values.len() == 1).then(|| self.values[0].as_str())
    }

    fn field_values(&self, custom_id: &str) -> Option<&[String]> {
        self.fields
            .iter()
            .find(|field| field.custom_id == custom_id)
            .map(|field| field.values.as_slice())
    }

    fn field_first(&self, custom_id: &str) -> Option<&str> {
        self.field_values(custom_id)?.first().map(String::as_str)
    }

    fn confirmed(&self) -> bool {
        self.field_first("confirmation") == Some("confirm")
    }

    fn reply(
        &self,
        content: impl Into<String>,
        ephemeral: bool,
        rows: Vec<MessageComponentRow>,
    ) -> CoreActionData {
        CoreActionData::ReplyInteraction {
            interaction_id: self.interaction_id.to_owned(),
            content: content.into(),
            ephemeral,
            rows,
        }
    }

    fn update(&self, content: impl Into<String>, rows: Vec<MessageComponentRow>) -> CoreActionData {
        CoreActionData::UpdateInteraction {
            interaction_id: self.interaction_id.to_owned(),
            content: content.into(),
            rows,
        }
    }

    fn edit_reply(
        &self,
        content: impl Into<String>,
        rows: Vec<MessageComponentRow>,
    ) -> CoreActionData {
        CoreActionData::EditInteractionReply {
            interaction_id: self.interaction_id.to_owned(),
            content: content.into(),
            rows,
        }
    }

    fn show_modal(&self, modal: ModalDefinition) -> CoreActionData {
        CoreActionData::ShowModal {
            interaction_id: self.interaction_id.to_owned(),
            modal,
        }
    }

    fn error_action(&self, content: impl Into<String>) -> CoreActionData {
        if self.is_modal {
            self.edit_reply(content, Vec::new())
        } else {
            self.reply(content, true, Vec::new())
        }
    }
}

fn home_rows() -> Vec<MessageComponentRow> {
    vec![row(vec![
        button("settings:notifications", "通知設定", ButtonStyle::Primary),
        button(
            "settings:roles",
            "通知ロール共通設定",
            ButtonStyle::Secondary,
        ),
        button(
            "settings:maintainers",
            "Botメンテナー設定",
            ButtonStyle::Secondary,
        ),
    ])]
}

fn notification_rows() -> Vec<MessageComponentRow> {
    vec![
        row(vec![MessageComponentData::StringSelect {
            custom_id: "settings:notification-category".to_owned(),
            placeholder: "通知種別を選択".to_owned(),
            options: notification_categories()
                .into_iter()
                .map(|category| {
                    option(
                        notification_label(category),
                        notification_key(category),
                        None,
                        false,
                    )
                })
                .collect(),
            min_values: 1,
            max_values: 1,
        }]),
        row(vec![button(
            "settings:home",
            "設定メニューへ戻る",
            ButtonStyle::Secondary,
        )]),
    ]
}

fn notification_back_rows(category: NotificationCategory) -> Vec<MessageComponentRow> {
    vec![row(vec![button(
        format!(
            "settings:notification-page:{}:0",
            notification_key(category)
        ),
        "通知設定へ戻る",
        ButtonStyle::Primary,
    )])]
}

fn role_back_rows() -> Vec<MessageComponentRow> {
    vec![row(vec![button(
        "settings:roles",
        "通知ロール設定へ戻る",
        ButtonStyle::Primary,
    )])]
}

fn maintainer_back_rows() -> Vec<MessageComponentRow> {
    vec![row(vec![button(
        "settings:maintainers",
        "Botメンテナー設定へ戻る",
        ButtonStyle::Primary,
    )])]
}

fn confirm_modal(
    custom_id: String,
    title: &str,
    content: String,
    confirmation_label: &str,
) -> ModalDefinition {
    ModalDefinition {
        custom_id,
        title: title.to_owned(),
        components: vec![
            ModalComponentData::TextDisplay { content },
            ModalComponentData::StringSelect {
                custom_id: "confirmation".to_owned(),
                label: "確認".to_owned(),
                description: None,
                options: vec![option(confirmation_label, "confirm", None, true)],
                min_values: 1,
                max_values: 1,
            },
        ],
    }
}

fn role_create_modal() -> ModalDefinition {
    ModalDefinition {
        custom_id: "settings:modal:role-create".to_owned(),
        title: "通知ロールを新規作成".to_owned(),
        components: vec![
            ModalComponentData::TextDisplay {
                content: "権限を持たない通知用ロールを作成し、通知ロール一覧へ登録します。"
                    .to_owned(),
            },
            ModalComponentData::TextInput {
                custom_id: "name".to_owned(),
                label: "ロール名".to_owned(),
                description: None,
                style: ModalTextInputStyle::Short,
                required: true,
                value: None,
                placeholder: Some("更新通知".to_owned()),
                max_length: Some(100),
            },
        ],
    }
}

fn row(components: Vec<MessageComponentData>) -> MessageComponentRow {
    MessageComponentRow { components }
}

fn button(
    custom_id: impl Into<String>,
    label: impl Into<String>,
    style: ButtonStyle,
) -> MessageComponentData {
    MessageComponentData::Button {
        custom_id: custom_id.into(),
        label: label.into(),
        style,
        disabled: false,
    }
}

fn option(label: &str, value: &str, description: Option<&str>, default: bool) -> SelectOption {
    SelectOption {
        label: truncate(label, 100),
        value: value.to_owned(),
        description: description.map(|value| truncate(value, 100)),
        default,
    }
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn notification_categories() -> [NotificationCategory; 5] {
    [
        NotificationCategory::Skd,
        NotificationCategory::Notice,
        NotificationCategory::Ad,
        NotificationCategory::UpdateAndroid,
        NotificationCategory::UpdateIos,
    ]
}

fn notification_key(category: NotificationCategory) -> &'static str {
    match category {
        NotificationCategory::Skd => "skd",
        NotificationCategory::Ad => "ad",
        NotificationCategory::Notice => "notice",
        NotificationCategory::UpdateAndroid => "android",
        NotificationCategory::UpdateIos => "ios",
    }
}

fn notification_label(category: NotificationCategory) -> &'static str {
    match category {
        NotificationCategory::Skd => "スケジュール更新通知",
        NotificationCategory::Ad => "ad更新通知",
        NotificationCategory::Notice => "notice更新通知",
        NotificationCategory::UpdateAndroid => "Android版更新通知",
        NotificationCategory::UpdateIos => "iOS版更新通知",
    }
}

fn parse_notification_category(value: &str) -> Option<NotificationCategory> {
    match value {
        "skd" => Some(NotificationCategory::Skd),
        "ad" => Some(NotificationCategory::Ad),
        "notice" => Some(NotificationCategory::Notice),
        "android" => Some(NotificationCategory::UpdateAndroid),
        "ios" => Some(NotificationCategory::UpdateIos),
        _ => None,
    }
}

fn parse_category_page(custom_id: &str) -> Option<(NotificationCategory, usize)> {
    let target = custom_id.strip_prefix("settings:notification-page:")?;
    let mut parts = target.split(':');
    let category = parse_notification_category(parts.next()?)?;
    let page = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((category, page))
}

fn parse_category_action(custom_id: &str, prefix: &str) -> Option<NotificationCategory> {
    parse_notification_category(custom_id.strip_prefix(prefix)?)
}

fn parse_page(custom_id: &str, prefix: &str) -> Option<usize> {
    custom_id.strip_prefix(prefix)?.parse().ok()
}

fn role_error_message(code: &str) -> &'static str {
    match code {
        "role_has_permissions" => "❌ 権限を持つロールは通知ロールへ追加できません。",
        "role_not_manageable" => "❌ Botが管理できないロールは通知ロールへ追加できません。",
        "role_unavailable" => "❌ 指定したロールが見つかりません。",
        "notification-role-limit-reached" => "❌ 通知ロールは最大9件です。",
        _ => "❌ 通知ロール設定の保存に失敗しました。",
    }
}

fn valid_snowflake(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn valid_subject_id(value: &str) -> bool {
    (17..=20).contains(&value.len())
        && value.chars().all(|character| character.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| number > 0)
}

fn message(channel_id: &str, content: impl Into<String>) -> CommandOutput {
    CommandOutput::single(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}
