# Storage・更新通知実装

## 構成

```text
POST /event-update
→ TypeScript HTTP境界（Secret・形式・16 KiB上限）
→ NativeCore.submitDetection
→ Rust NotificationService
→ StorageServiceへ送信予定を保存
→ sendNotification / editMessage
→ Discord Adapter
→ actionResult
→ StorageServiceへ結果を保存
```

通知設定と配送履歴の正本はprivate GitHub Repositoryである。Local memoryは復元可能なSnapshotとしてだけ使う。

## 設定

- `GITHUB_DATA_OWNER`
- `GITHUB_DATA_REPO`
- `GITHUB_DATA_BRANCH`（省略時`main`）
- `GITHUB_DATA_TOKEN`
- `EVENT_UPDATE_SECRET`（未設定ならHTTP Serverを起動しない）
- `EVENT_UPDATE_PORT`（省略時`PORT`、それもなければ3000）
- `EVENT_UPDATE_HOST`（省略時`0.0.0.0`）

GitHubのowner、repository、tokenは3つ揃っている場合だけ有効になる。通知HTTPを有効にする場合はStorage設定も必須である。

## StorageService

`crates/kbc-core/src/storage.rs`が共有`HttpService`を利用してGitHub Contents APIを扱う。

- Repositoryがprivateで、archived/disabledでないことを確認する。
- branchと`meta.json`のschemaを確認する。
- `config/guilds/*.json`を起動時に復元する。
- Bot全体で共通の`config/maintainers.json`を起動時に復元する。ファイルがまだ存在しない場合は空設定として扱う。
- Guild設定には通知先に加え、最大9件の通知用Role、単一のRole選択パネル、通知先別mention Role、最大9件のSKD関連サイトURLを保存する。追加fieldはSerde defaultで旧設定と互換にする。
- 1文書256 KiB、1Directory 999 Fileを上限とする。
- Writeは1秒間隔で直列化する。
- SHA付き更新で競合上書きを防ぐ。
- `push`設定のSHA競合時は最新版を再読込し、変更を再適用して1回だけ再試行する。
- 応答消失時は保存内容を再読込し、一致した場合だけ成功とする。
- Rate limit検知後は60秒のcooldownを置く。

`o.push`は固定管理者ID、登録ユーザーID、実行者の所持ロールIDを照合し、Storage保存成功後だけ登録・解除成功を返信する。同じ登録の再実行は重複もWriteも発生させない。

`o.maint maintainer`は固定Bot管理者だけが実行できる。最大64件のユーザーID・ロールIDを`config/maintainers.json`へ保存し、SHA競合時はGuild設定と同じく最新版へ変更を再適用して1回だけ再試行する。`list`のGuild member解決はDiscord固有処理としてAdapterへ有限Actionを依頼し、権限と表示内容の判断はCoreに残す。

`o.maint role`、`o.maint pushsetting`、`o.push ... role:<ID>`、`o.push skd url <URL> add|del`は固定Bot管理者またはBotメンテナーが実行できる。通知用Roleの登録時はAdapterで権限ゼロ・Bot管理可能を確認する。選択肢からRoleを削除した場合は全Subscriptionのmention設定からも同じIDを除くが、既にMemberへ付与済みのRoleは一括解除しない。

Role選択パネルはGuildごとに1件だけ保存する。別Channelで再設置した場合は新Messageを正本として保存した後、旧MessageのReactionを全削除し「通知設定は移動しました」へ編集する。選択肢更新時は本文と番号Reactionを再構築する。

## NotificationService

外部Eventは`version: 1`、Event ID、category、phase、検知時刻、schedule type、任意のsourceを検証する。SKDの`ready` Eventではbefore/afterの40桁Git SHA、`raw/<type>_<timestamp>.tsv`、MD5を検証する。

受付は最大4件、配送本体は1件ずつ処理する。Discord Action結果は30秒を上限に待つ。初回投稿はEvent IDとChannel IDから安定したnonceを作り、`enforceNonce: true`で送る。

配送状態は次の順で保存する。

```text
pending
→ attempting（Discord送信前に保存）
→ sent（actionResult成功とmessage IDを保存）
```

`attempting`のまま残った投稿は結果不明であり、自動再投稿せず`reconciliation-required`を返す。message IDが確定済みの編集失敗は、同じEventの再受信時に再編集する。

SKD詳細は`o.skd`と同じParser、差分、名称DataSource、Formatterを使う。生成した本文をEvent Recordへ保存してから、gatya、sale、item、mission、変更、関連サイトの順で配送する。関連サイトの先頭は従来のKBC履歴URLであり、`o.skd`実行時と通知配送時に対象Guildの追加URLを末尾へ差し込む。`o.skd`は追加URLを表示しても通常の`sendMessage`を使い、通知Roleをmentionしない。

通知Role mentionは初回の`sendNotification`だけで`allowedRoleIds`を明示する。最終本文への編集とSKD詳細Messageではmentionを許可しないため、1回の更新で重複mentionを発生させない。

## Local設定

`npm run setup:local`は既存V2設定を優先しつつ、旧RepositoryからDiscord Token、GitHub Token、Webhook Secretを値を表示せずに移す。旧版の`WORKFLOW_GITHUB_TOKEN`はLocal確認用の`GITHUB_DATA_TOKEN`として利用できる。

実在しない正常Eventを送る試験は、永続履歴やDiscord Channelを汚すため行わない。不正Eventの拒否とHTTP状態をLocalで確認し、正常配送の最終確認は最初の実更新で行う。

Localと旧本環境が同じ設定Fileへ同時に書き込み、GitHubのSHA競合が発生することを実Discordで確認した。有限な再試行後は登録に成功した。通常運用では旧版とV2を常時並行稼働させない。

## 本環境

2026-09-22にNorthflankの既存Serviceのbuild sourceをpublic code Repository `sinsuirakv0/KBC-rakv0-discord-bot-v2`へ変更した。通知設定と配送履歴を保存するprivate Data Repositoryとは別である。自動deployを一時停止してDocker build成功を確認した後、commit `976c7c3`を手動deployし、旧Container終了後にV2が起動した。公開URLとPort 3000、既存のDiscord・GitHub Storage・Event Update環境変数は維持した。

起動後はDiscord login、`/health/live`の200 alive、`/health`の200 ready、Northflank上の1/1 Runningを確認した。CI/CDはV2 Repositoryを対象として再有効化した。
