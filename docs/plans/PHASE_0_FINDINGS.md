# Phase 0 調査結果とV2初期設計案

- 状態: 完了
- 調査日: 2026-09-20
- 現行構成: [`../architecture/CURRENT_SYSTEM.md`](../architecture/CURRENT_SYSTEM.md)
- 基本計画: [`RUST_CORE_V2.md`](RUST_CORE_V2.md)

## 1. 結論

既存のV2方針である、

```text
TypeScript = Discord Adapter
Rust = Bot Core
KBC Protocol = 明示的な境界
```

は、現行コードの問題に適合している。

ただしWorkspace Skeletonを作る前に、次の4点を設計として確定する必要がある。

1. `ActionResult`を含むEvent / Action往復契約
2. Reaction選択とPaginationを再開できるSession model
3. N-APIだけを担当する薄いBridge crateの配置
4. StorageとNotificationを移す順序

これらを曖昧にしたままCommand移植を始めると、TypeScriptへCommand stateが戻るか、Commandごとの専用N-API関数が増える。

## 2. 設計上の問題一覧

### P0: Protocolが対話処理を完結できる粒度まで未確定

現行の対話処理は、Command内で次を直接awaitする。

```text
Message送信
→ Reaction追加
→ Reaction待機
→ Reaction削除
→ Message編集
```

V2ではTypeScriptがこの状態を持たず、Gateway EventをCoreへ渡し、CoreがSessionを再開する必要がある。

そのため`CoreAction`を出すだけでは不足し、Discord操作の成功・失敗と、送信時に確定した`messageId`を`ActionResult`としてCoreへ戻す必要がある。

### P0: N-API Bridgeの責務がcrate構成上で未確定

`kbc-protocol`へN-API型を直接置くと、Protocol定義がNode固有型に依存する。`kbc-core`へ置くと、Bot本体がN-API lifecycleへ依存する。

推奨候補は次の3 crateである。

```text
crates/
├─ kbc-protocol/  # versioned DTO。napiへ依存しない
├─ kbc-core/      # RuntimeとDomain。napiへ依存しない
└─ kbc-node/      # N-API変換とNode向けLifecycleだけ
```

1 crate増えるが、Protocol・Core・Transportの依存方向が明確になる。重要Architecture変更として、採用前に`docs/decisions/`へ記録する。

### P0: 現行Phase順ではStorage依存Featureの移行順が曖昧

現在の計画は複雑CommandをPhase 12、StorageをPhase 13としている。一方、`push`とNotificationは、送信前保存・結果不明保留・再起動復旧を仕様として要求する。

Storage一般を早期に作り込む必要はないが、Notificationを移す前にはGitHub保存の契約を先に移す必要がある。

### P1: HTTPとCache PolicyがCommandごとに重複

現行ではtimeout、status確認、条件付きGET、hash比較、stale fallbackが複数Data Sourceに別実装されている。

V2では共通`HttpService`で次だけを統一する。

- timeout
- 最大body size
- bounded concurrency
- status分類
- conditional request用validator
- cancellation
- metrics hook

解析済みSnapshotのCache方針はFeature Repositoryへ残す。全Responseを自動Cacheする巨大Frameworkにはしない。

### P1: 有限性が保証されていない

現行で特に注意が必要なもの:

- motion待機列に容量上限がない
- Asset存在確認Cacheに最大件数がない
- 異なるNotification eventIdは同時に増え得る
- 一部Data Sourceが取得対象数に応じて`Promise.all`する
- Action出力とEvent入力のQueueがV2計画上まだ具体化されていない

V2ではEvent queue、Action queue、Task queue、Session数、Cache件数、HTTP同時数を必ず有限にする。具体値は想定だけで決めず、保守的な初期値と計測結果で調整する。

### P1: NotificationがCommand内部実装へ依存

Schedule notificationは`sale`・`gatya`・`item`のparser、domain、formatterを直接importする。表示互換には有効だが、Command UIとSchedule domainの境界が曖昧である。

V2では次へ分離する。

```text
Schedule Repository / Parser / Domain
          ├─ sale / gatya / item Command
          └─ Notification formatter
```

Command同士を依存させず、共有Domainへ依存させる。

### P1: Native Rustと本番motion経路が二重化

現行Native Rustは比較Commandだけで使われ、本番WorkerはJavaScript engineを使う。V2で旧N-API APIをそのまま温存すると、KBC Protocol Bridgeとmotion専用Bridgeが並存する。

Phase 0時点では現行Rust motion codeを`kbc-core`内部moduleへ移す案を推奨した。その後のBenchmarkでMotion評価以外が支配的と判明したため、この案は`docs/decisions/MOTION_RENDERING_V2.md`の再設計判断に置き換えた。Command専用N-APIを追加しない方針は維持する。

### P1: LifecycleとReadinessが分散

現行はStorage再試行、Discord login、任意HTTP serverが別々に開始される。Storage未設定でも再試行が続く。Shutdown処理も統合されていない。

V2では`AppRuntime`がCore側ServiceとQueueを所有し、初期化状態を明示する。TypeScriptはDiscord ClientとCore Bridgeの開始・停止だけを調整する。

### P2: Observabilityがconsole log中心

Command、Action、Task、HTTP、Storageを一つのIDで追跡できない。Protocol導入時に`RequestId`と`ActionId`を入れ、詳細Metricsは実需要が出てから追加する。

### P2: Configurationが分散し、一部値がcode固定

URL、timeout、管理者ID、Queue条件がFeature別configへ散在している。V2ではRust側`Configuration`へ集約するが、全設定を自由変更可能にする必要はない。互換性上固定すべきPrefixなどは型付き定数のまま保持する。

## 3. V2計画との差異

| 項目 | 現行 | V2計画 | 設計上の対応 |
| --- | --- | --- | --- |
| Command logic | ほぼTypeScript | Rust Core | 段階移行する |
| Discord依存 | Adapterへ概ね集約 | 薄いAdapter | 方針を維持する |
| Interactive state | TypeScriptのawait stack | SessionManager | Protocolを先に確定する |
| Runtime | Command関数の直接呼出し | AppRuntime | bounded Event queueを追加する |
| Action出力 | CommandContextから直接Discord | ActionBus | `ActionResult`往復を必須にする |
| HTTP | Command別fetch | HttpService | Policyだけ共通化する |
| Cache | Featureごとに不統一 | CacheManager候補 | 必要なFeatureから導入する |
| Task | motion専用直列Queue | TaskRuntime | motion移行直前に汎用化する |
| Resource | motion同時1件のみ | ResourceManager | Queue容量とpermitを追加する |
| Storage | GitHub Contents API | StorageService | Notificationより先に移す |
| Native Rust | motion比較のみ | Bot本体 | motion codeをCore内部へ統合する |
| HTTP ingress | Node HTTP Server | RustがHTTPを担当 | Rust側Serviceを推奨する |
| Metrics | console中心 | Metrics | IDと最小Timingから始める |

## 4. 初期Protocol案

これはPhase 2実装前の設計候補であり、まだ確定ではない。

### 4.1 CoreEvent

初期Variant:

```text
MessageCreate
ReactionAdd
ActionResult
```

必要になった時点で追加するVariant:

```text
ButtonInteraction
TaskCancellation
```

`Shutdown`はEventにせず、Lifecycle APIの`shutdown()`として扱う。

`MessageCreate`はDiscord Objectではなく、最低限次を持つ。

```text
eventId
guildId?
channelId
messageId
userId
memberRoleIds
content
```

Discord SnowflakeはNumberへ変換せずStringで保持する。

`ReactionAdd`候補:

```text
eventId
guildId?
channelId
messageId
userId
emoji
```

`ActionResult`候補:

```text
actionId
success(messageId?など)
または
failure(code, retryable)
```

内部例外やStack TraceはProtocolへ載せない。

### 4.2 CoreAction

現行機能を移すための最小Variant:

```text
SendMessage
EditMessage
SendAttachment
AddReaction
ClearReactions
```

各Actionは`actionId`を持つ。Command実行全体を追跡する場合は`requestId`を持たせる。`sessionId`と`taskId`は該当機能にだけ持たせ、全Variantへnullable fieldとして追加しない。

Attachmentはbase64文字列にせず、N-API BridgeでBufferとして渡す。最大sizeをCoreとAdapterの両方で検査し、大きなBufferの不要copyを計測する。

### 4.3 Lifecycle API

候補:

```text
createCore(config)
getRuntimeInfo()
submitEvent(event)
nextAction()
shutdown()
```

- `submitEvent`はbounded queueへ投入し、無制限にMemoryへ蓄積しない。
- `nextAction`はshutdown時に終了可能とする。
- TypeScriptはActionを実行した後、必ず`ActionResult`を返す。
- 起動時にProtocol versionとCore versionを検査する。

## 5. Runtime所有権案

```text
AppRuntime
├─ CommandRuntime
├─ SessionManager
├─ TaskRuntime
├─ ResourceManager
├─ Services
└─ ActionBus
```

`ServiceHub`は文字列KeyのService Locatorにせず、具体型を持つ`Services` structとして解釈する。

```text
Services
├─ configuration
├─ clock
├─ http
├─ metrics
├─ storage      # 必要になったPhaseで追加
└─ repositories # 必要になったFeatureだけ追加
```

Commandへ`Services`全体を無条件に渡さず、必要なCapabilityだけを参照させる。`Arc<Mutex<_>>`をGlobal Stateとして共有しない。

## 6. Session設計案

Reaction選択を移す時点で最低限次を導入する。

```text
SessionId
OwnerUserId
GuildId / ChannelId
BotMessageId
Expiry
State enum
```

Stateは大量のbooleanではなく、用途別enumで表す。

```text
SearchSelection
Pagination
FileSelection
```

処理:

```text
Command
→ SendMessage Action
→ ActionResult(messageId)
→ Session登録
→ AddReaction Action
→ ReactionAdd Event
→ Session state更新
→ Edit / Clear / Result Action
```

Sessionは最大件数と期限を持ち、期限切れcleanupをRuntimeが所有する。TypeScriptのCollectorへ意味を持たせない。

## 7. Task・Resource設計案

最初から汎用Schedulerを作らない。motion移行直前に次だけ実装する。

```text
TaskRuntime
├─ bounded queue
├─ cancellation token
├─ timeout
├─ progress event
└─ cleanup guard

ResourceManager
├─ CpuHeavyPermit
└─ ExternalProcessPermit
```

初期のmotionはCPU-heavy permit 1、External-process permit 1を基本候補とする。ただし確定値はNorthflank上の計測で見直す。Queue満杯時は待ち続けず、利用者へbusyを返せる設計にする。

## 8. 推奨移行順

既存Phaseを全面的に置き換えず、Featureの依存順を次のように具体化する。

1. Workspaceと薄いN-API Bridge
2. Version付きProtocolとDummy roundtrip
3. AppRuntime、bounded Event / Action bus、shutdown
4. Discord AdapterとActionResult
5. Rust Command parser・Registryと`ping`
6. `content/responses`・`content/help`のFile loader
7. Configuration、Clock、最小HttpService
8. `item`で外部Data・validation・chunkingを確認
9. `sale`・`gatya`と共有Schedule domain
10. `eventdata`
11. `st`でvalidated Snapshot Cacheを確認
12. SessionManager導入後に`ut`・`tut`の検索・File選択
13. TaskRuntime・ResourceManager導入後にmotion
14. GitHub Storage
15. Notification ingress、`push`、`skd`
16. 観測、実測、不要Migration code整理

Storageは早期に一般化せず、Notificationの直前に現行契約を理解して移す。

## 9. 改善提案の比較

### 9.1 Bridge crateを分離する

- 現在案: `kbc-protocol`と`kbc-core`の2 crate候補
- 問題: N-API依存の置き場所が曖昧
- 代替案: 薄い`kbc-node`を追加
- 利点: CoreとProtocolがNode非依存、境界testが容易
- 欠点: crateとbuild設定が1つ増える
- 影響範囲: Workspace、Docker、型生成、N-API build

### 9.2 ActionResultをPhase 2から入れる

- 現在案: Event / Action候補には記載があるが往復手順は未確定
- 問題: messageIdを必要とするSessionとProgressを移せない
- 代替案: 最初のSmokeからActionResultを含める
- 利点: 後からProtocolを破壊せず、Discord失敗もCoreで扱える
- 欠点: Dummy roundtripが少し増える
- 影響範囲: Protocol、ActionBus、Discord Adapter、Session

### 9.3 ServiceHubを具体型へ限定する

- 現在案: `ServiceHub`
- 問題: 汎用Locator化すると依存が隠れ、Global State化しやすい
- 代替案: `Services` concrete structと狭いCapability参照
- 利点: Ownershipと依存がCompilerで明確
- 欠点: Service追加時にconstructor更新が必要
- 影響範囲: AppRuntime、CommandContext、各Service

### 9.4 Native motionをCore内部へ統合する（Phase 0案・再設計で置換済み）

- 現在案: 現行motion専用N-APIと新KBC Protocolが並存し得る
- 問題: EngineとBridgeが二重化する
- 代替案: Phase 0時点では現行Rust計算部をCore moduleへ移す案だったが、現在は旧コードを参照だけに使い、Rust中心の描画・encode pipelineを再設計する
- 利点: 本番経路が1つになり、Command専用N-APIを増やさない
- 欠点: 新Rasterizerとencode経路の互換性・性能確認が必要
- 影響範囲: motion、TaskRuntime、Docker、compatibility smoke

## 10. Phase 0の確定事項と後続調査

### ユーザー確定事項

1. `archive/legacy-v1`にだけ存在する`gmtt`・`save`・`stnp`・監視fallback等は、V2の移行対象へ含めない。
2. 管理者用`utbench`はテスト用途のため、V2へ移植しない。
3. Phase 1へ進み、N-API Bridgeを分離したWorkspaceを構築する。

### 調査・技術判断を続ける事項

1. `kbc-node`採用時の型生成方法
2. N-API Bufferのcopy回数とAttachment memory上限
3. Rust内部HTTP ServerのNorthflank health・shutdown連携
4. 現行Rust motionとJavaScript motionのcompatibility範囲
5. Northflank上のidle RAM、motion peak RAM、Task latency

後続調査は各Phaseの実装判断として継続し、重要な変更は`docs/decisions/`へ記録する。Phase 0は完了とする。
