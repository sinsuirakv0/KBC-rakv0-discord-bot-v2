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
- 1文書256 KiB、1Directory 999 Fileを上限とする。
- Writeは1秒間隔で直列化する。
- SHA付き更新で競合上書きを防ぐ。
- `push`設定のSHA競合時は最新版を再読込し、変更を再適用して1回だけ再試行する。
- 応答消失時は保存内容を再読込し、一致した場合だけ成功とする。
- Rate limit検知後は60秒のcooldownを置く。

`o.push`は固定管理者IDを照合し、Storage保存成功後だけ登録・解除成功を返信する。同じ登録の再実行は重複もWriteも発生させない。

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

SKD詳細は`o.skd`と同じParser、差分、名称DataSource、Formatterを使う。生成した本文をEvent Recordへ保存してから、gatya、sale、item、mission、変更、KBCリンクの順で配送する。

## Local設定

`npm run setup:local`は既存V2設定を優先しつつ、旧RepositoryからDiscord Token、GitHub Token、Webhook Secretを値を表示せずに移す。旧版の`WORKFLOW_GITHUB_TOKEN`はLocal確認用の`GITHUB_DATA_TOKEN`として利用できる。

実在しない正常Eventを送る試験は、永続履歴やDiscord Channelを汚すため行わない。不正Eventの拒否とHTTP状態をLocalで確認し、正常配送の最終確認は最初の実更新で行う。

Localと旧本環境が同じ設定Fileへ同時に書き込み、GitHubのSHA競合が発生することを実Discordで確認した。有限な再試行後は登録に成功した。通常運用では旧版とV2を常時並行稼働させない。
