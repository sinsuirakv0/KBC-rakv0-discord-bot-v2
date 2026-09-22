# 現行システム調査

- 状態: Phase 0 調査結果
- 調査日: 2026-09-20
- 参照元: `D:\KBC\KBC-rakv0-discord-bot`
- 参照条件: 旧プロジェクトは読み取り専用とし、変更しない

## 1. 調査範囲

現行リポジトリについて、次を確認した。

- `AGENTS.md`
- `docs/requirements/`
- `docs/decisions/`
- `docs/implementation/`
- 起動処理、Command基盤、Discord Adapter
- HTTP Data SourceとCache
- Notification HTTP Server
- GitHub永続保存
- motion Worker、FFmpeg、Native Rust
- DockerとNorthflank前提
- `archive/legacy-v1/`の機能一覧

仕様の優先順位は、現行リポジトリの規約どおり次の順とする。

```text
requirements
→ decisions
→ implementation
→ active source code
→ archive/legacy-v1
```

`archive/legacy-v1`は、現行稼働機能ではなく履歴資料として区別する。

## 2. 現行Command一覧

Prefixは固定の`o.`。Command名は大文字小文字を区別せず、全CommandがGuild限定で、DMでは応答しない。

### 2.1 静的Command

`content/responses/*.txt`を起動時に列挙し、ファイル名をCommand名、本文を返信内容として登録する。

| Command | 主な役割 |
| --- | --- |
| `ping` | 疎通確認 |
| `home` | KBC関連ページへのリンク |
| `skb` | 静的返信 |
| `asset` | Asset関連ページへのリンク |
| `skdsite` | Eventサイトへのリンク |

個別の`content/help/<command>.txt`がなければ、静的返信本文をhelpとして使用する。

### 2.2 動的Command

| Command | 主な処理 | 対話・高負荷・副作用 |
| --- | --- | --- |
| `help` | help index表示 | File読込 |
| `sale` | sale一覧、詳細、名前検索、JSON、Raw | 4～9件でReaction選択 |
| `gatya` | gatya一覧、ID・series検索、JSON、Raw | 複数Data Source |
| `item` | item一覧、詳細、JSON、Raw | 複数Data Source |
| `st` | Map・Stage検索 | Reaction選択、Pagination、10分Cache |
| `tut` | 敵検索、File、Origin、Motion | Session相当のReaction処理、Worker、FFmpeg |
| `ut` | 味方検索、File、Origin、Motion | Session相当のReaction処理、Worker、FFmpeg |
| `eventdata` | 公式・KBC URL、平文・暗号化添付 | 外部認証、最大5件取得、AES暗号化 |
| `push` | Notification購読設定 | 固定管理者限定、GitHubへ永続保存 |
| `skd` | 保存済みSchedule履歴の差分表示 | Git履歴取得、複数TSV解析 |
| `utbench` | LegacyとNative Rustのmotion比較 | 固定管理者限定の開発診断Command。V2移行対象外 |

`utbench`は実行Registryへ登録されているが、一般向けrequirementsとhelpはない。

## 3. 現行Architecture Map

### 3.1 通常Command

```text
Discord Gateway
  ↓ messageCreate
src/discord/message-handler.ts
  ├─ Discord MessageをCommandContextへ変換
  ├─ Bot投稿を除外
  └─ 管理者ID判定
  ↓
parseCommandInput
  ↓
CommandRegistry
  ↓
Command.execute
  ├─ parser / domain / formatter
  ├─ Command固有Data Source
  └─ InteractiveCommandOutput
       ↓
     discord.js send / edit / react / clear / awaitReactions
```

Discord.js依存は主に`src/discord/`と`src/index.ts`へ閉じている。一方、Commandが送信・編集・Reaction待機を順番に直接制御するため、Botの処理状態はTypeScript側の非同期呼び出しスタックに残る。

### 3.2 Notification

```text
External event workflow
  ↓ POST /event-update
Node HTTP Server
  ↓ parse / authenticate
DetectionService
  ├─ NotificationStore
  │    ↓
  │  JsonStore
  │    ↓
  │  GitHub Contents API
  ├─ Schedule diff builder
  │    ├─ Git tree / raw TSV
  │    └─ sale / gatya / itemのparser・domain・formatterを再利用
  └─ Discord NotificationTransport
       ├─ send
       └─ edit
```

送信直前に`attempting`を保存し、送信結果が不明な場合は自動再投稿せず、手動確認まで保留する。GitHubとDiscordを同一Transactionにできないため、重複投稿回避を優先したOutbox相当の設計になっている。

### 3.3 Motion

```text
ut / tut
  ↓
共有の直列待機列
  ↓
Asset取得
  ↓
worker_threads Worker
  ├─ JavaScript motion-engine
  ├─ @napi-rs/canvas
  └─ 全対象Frameの範囲計測と描画
       ↓ raw RGBA stream
FFmpeg child process
  ↓
MP4 / GIF / PNG
  ↓
Discord Attachment
```

- `ut`と`tut`で同時実行数1の待機列を共有する。
- 動画FrameはWorkerからFFmpegへstreamし、全FrameをMemoryへ保持しない。
- FFmpeg thread数は1。
- 無進捗60秒、全体10分で停止する。
- 待機列には容量上限がない。

### 3.4 Native Rust

現行Native Rustは`native/motion-core`だけである。

- imgcut、mamodel、maanimの解析
- motion pose計算
- draw packet生成
- 複数Frameのcompact packet生成
- N-API公開

ただし通常の`ut`・`tut`はJavaScript版motion engineを使っている。Native Rustは`utbench`の比較経路だけで使用され、現行本番描画経路にはまだ採用されていない。

Dockerは常にNative Rustをbuildして`.node`へコピーするため、通常Commandで未使用でもbuild costとimage構成には含まれる。

## 4. Data SourceとCache

### 4.1 HTTP取得元

| 分類 | 主な取得元 |
| --- | --- |
| Schedule | `KBC-rakv0-event`のraw GitHub data |
| Game Asset metadata | `KBC-rakv0-assets`のraw GitHub data |
| Alias data | `Sugar2550/omoroirie` |
| eventdata | PONOS account・auth・event server、KBC API |
| 履歴 | GitHub Git tree APIとcommit固定raw URL |
| 永続保存 | private GitHub repositoryのContents API |
| Discord表示先 | JDB、KBC、各種Event siteへのURL |

### 4.2 現行Cache

| 対象 | 方針 |
| --- | --- |
| `st`検索Data | 全関連CSVを1 Snapshotとして10分保持、ETag・Last-Modified・hash、stale fallback |
| `tut`検索Data | TSVとJSONを1 Snapshotとして10分保持、条件付きGET、stale fallback |
| `ut`検索Data | character indexとunitbuyを別々に10分保持、条件付きGET、stale fallback |
| Asset存在確認 | Path単位で10分保持 |
| eventdata token | 10分保持し、同時発行を1 Promiseへ集約 |
| `sale` / `gatya` / `item` | Command実行ごとに取得し、共有Cacheなし |
| motion元Asset・生成物 | Cacheしない |

HTTP timeout、status検査、body検証は存在するが、実装は各Commandに分散している。共通の接続数制限、全体Network permit、最大bodyの共通Policyはない。

## 5. Storage

正本はprivate GitHub repositoryであり、Local diskとMemoryは永続保存として扱わない。

```text
GitHubDataRepository
  ├─ repository / branch / private確認
  ├─ Contents API read / list / write
  ├─ SHAによる競合検出
  ├─ write直列化
  └─ 最低1秒のwrite間隔

JsonStore
  ├─ schema検証
  └─ process内の全JSON updateを直列化

GuildSettingsStore
  ├─ notification subscriptions
  └─ health maintainer role用field

NotificationStore
  └─ event単位のdelivery state
```

制約:

- 1 processだけが書き込む前提。
- GitHub SHAは分散Lockではない。
- 1文書256 KiB。
- 設定directoryは1000件未満。
- 起動時の復元に失敗した場合は30秒以上空けて再試行する。
- 保存設定がない場合も現行`startStorage()`は再試行を継続する。

## 6. Background処理とConcurrency

現行active sourceに常駐する可能性がある処理は次のとおり。

- Discord Gateway
- 任意のNotification HTTP Server
- Storage初期化の再試行Timer
- Notification eventId単位のprocess内Queue
- Motion待機列
- Motion Worker thread
- FFmpeg child process
- Discord Reaction collector

`archive/legacy-v1`にあるGitHub Actions監視、Playwright screenshot、fallback workflow起動、Button操作、起動通知はactive sourceからは起動されない。

## 7. Dependency Map

```text
src/index.ts
├─ discord
├─ commands
├─ notifications
├─ storage
└─ config

commands
├─ command固有 parser / domain / formatter / data-source
├─ shared messaging / search / file-picker / motion
└─ config

notifications
├─ storage
├─ discord notification transport
└─ sale / gatya / itemの内部parser・domain・formatter

storage
├─ config
└─ notificationsの型

motion
├─ worker_threads
├─ @napi-rs/canvas
├─ ffmpeg-static
├─ JavaScript vendor engine
└─ benchmark時のみNative Rust
```

重要な結合:

- Notificationが`sale`・`gatya`・`item` Command内部へ直接依存する。
- `tut`のAsset取得が`ut` Data Sourceを再利用する。
- StorageがNotification型へ依存し、NotificationがStorage実装へ依存する。
- CommandContextがDiscord操作の手順をCommandへ公開している。

## 8. Runtime・Deployment前提

| 項目 | 現行 |
| --- | --- |
| Node | 22 bookworm-slim |
| TypeScript | 5.5系、CommonJS、strict |
| Rust build image | Rust 1.89 bookworm |
| Discord | discord.js 14 |
| HTTP listen port | `EVENT_UPDATE_PORT` → `PORT` → 3000 |
| Health | `/health/live`と`/health` |
| Storage | private GitHub repository |
| Local disk | 一時Fileのみ |
| Heavy processing | Worker 1件 + FFmpeg 1 thread |
| 想定実績 | Northflank 0.1 vCPU / 256 MiBの記録あり |

現行資料のWindows測定ではmotionのRSSが高く、Node本体とFFmpegを含むNorthflank上のPeak memoryは未確定である。V2でも計測前に余裕があるとは仮定しない。

## 9. Archiveにだけ存在する機能

次は`archive/legacy-v1`には存在するが、active sourceのRegistryや起動処理には存在しない。

- `gmtt`
- `save`
- `stnp`
- `test`
- GitHub Actions監視とfallback実行
- Playwright screenshot
- Button Interaction
- 起動時の固定Channel通知

2026-09-20のユーザー決定により、これらはV2移行対象に含めない。

## 10. この調査で実行していないこと

- 旧プロジェクトのFile変更
- `npm test`やbuildによる`dist`更新
- Docker build
- Discord実接続
- Northflank実測
- 外部APIへの実通信

本書はSourceと既存資料の読み取り結果であり、実環境の性能値や外部Serviceの現在状態を保証するものではない。
