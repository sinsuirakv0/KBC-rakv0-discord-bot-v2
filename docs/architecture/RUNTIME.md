# KBC Core Runtime設計

## 1. 現在の実装範囲

非Motion Command移植完了時点では、入出力、Lifecycle、Command Runtime、Local Contentの不変Snapshot、共有HTTP Service、GitHub Storage、Notification Runtime、有限な複数Action出力、短時間の対話を扱うSession Managerを持つ。`ut`/`tut`用の検証済みRemote snapshotと有限なAsset存在Cacheは持つが、汎用Cache Framework、Task Runtime、Resource Managerはまだ実装しない。

```text
NativeCore (N-API)
└─ AppRuntime
   ├─ RuntimeConfig
   │  ├─ Content Catalog ──→ Command構築
   │  └─ HTTP上限 ────────→ HttpService
   ├─ Event sender ──→ bounded Event Queue (既定64件)
   ├─ Worker
   │  ├─ Event Queueの唯一のconsumer
   │  ├─ CommandRuntime
   │  │  ├─ Parser
   │  │  ├─ CommandRegistry
   │  │  ├─ Command
   │  │  ├─ shared HttpService
   │  │  └─ Clock
   │  ├─ Session Manager
   │  │  ├─ sendMessage結果待ちSession
   │  │  ├─ message IDに結び付いた有効Session
   │  │  └─ 最短期限TimerとCleanup
   │  └─ Action Queueの唯一のproducer
   ├─ ActionBus ─────→ bounded Action Queue receiver (既定16件)
   ├─ StorageService ─→ private GitHub Repository
   ├─ NotificationService
   │  ├─ 外部更新の検証と最大4件の受付
   │  ├─ Android・iOS Store Versionの独立監視
   │  ├─ 直列配送と永続Checkpoint
   │  └─ ActionResult待機（最大30秒）
   ├─ Shutdown signal
   └─ Worker JoinHandle
```

`AppRuntime::start()`がContent Catalogを読み、共有`HttpService`、`Clock`、`StorageService`を作り、各Commandへ必要な能力だけを注入してからRuntimeを開始する。CatalogやService全体を`CommandContext`へ渡さない。CommandRuntimeはWorkerへmoveされ、Eventを直列に処理する。`ActionBus`はAction受信側を所有し、Command WorkerとNotification Serviceが送信側を共有する。どちらも同じbounded Action Queueを通るため、通知用の無制限Bufferは作らない。

## 2. QueueとBackpressure

Event QueueとAction QueueにはTokioのbounded channelを使う。

| Queue | 既定容量 | 設定可能範囲 |
|---|---:|---:|
| Event | 64 | 1〜1024 |
| Action | 16 | 1〜1024 |

Event Queueが満杯の場合、`submit_event()`は空きができるまで待つ。Action Queueが満杯の場合、Workerも空きができるまで待つ。失敗や無制限Bufferingへ切り替えず、consumerの速度をproducerへ伝える。

容量、Content root path、HTTP同時実行数・timeout・Response上限は`RuntimeConfig`で起動時だけ変更できる。HTTPの既定値は同時4件、10秒、8 MiBであり、設定値には16 MiBの固定上限を設ける。8 MiBは`character-index.json`の実測625,735 byteとAsset添付を同じ共有Serviceで有限に扱うために設定した。

## 3. AppRuntime API

- `AppRuntime::start(config)`: Queue容量を検証し、Contentを読み込んだ後、Queue、停止通知、Workerを作る。
- `submit_event(event)`: Protocol Versionを検査し、Event Queueへ投入する。
- `next_action()`: Actionを1件待つ。停止後は`None`を返す。
- `shutdown()`: 停止を通知し、Command Worker、Store監視、Task Workerの終了を待つ。
- `is_shutdown()`: 現在の停止状態を返す。

N-APIでは`createCore()`が`AppRuntime::start()`を呼び、返された`NativeCore`が`submitEvent()`、`nextAction()`、`shutdown()`を公開する。

## 4. LifecycleとShutdown

`shutdown()`は冪等であり、複数callerから同時に呼ばれても同じWorker終了を待つ。最初の呼び出しが停止状態を確定して全待機処理へ通知する。

停止通知により、次を終了可能にする。

- Event Queueの空き待ち中の`submit_event()`
- Action待ち中の`next_action()`
- EventまたはAction Queue待ち中のWorker

停止開始後は新しいEventを受け付けない。現在は高速で予測可能な停止を優先し、Queue内の未処理Eventと未取得Actionをdrainしない。永続化やgraceful drainが必要になった場合は、要件と上限時間を定めて別途設計する。

## 5. 現在の通常Command経路

```text
messageCreate CoreEvent
→ submit_event
→ bounded Event Queue
→ Worker
→ CommandRuntime
→ Prefix Parser
→ CommandRegistry
→ MetadataによるGuild判定
→ Command
→ 最大32件のCommand出力
→ sendMessage CoreAction群
→ bounded Action Queue
→ next_action
```

複数Actionの先頭IDは従来どおり`action:<eventId>`、2件目以降は`action:<eventId>:<連番>`とする。Action Queueが満杯なら各Actionの投入時に待機するため、複数返信でもbackpressureを維持する。

現在登録されているCommandは、静的な`asset`、`home`、`ping`、`skb`、`skdsite`、`help`、外部データを使う`item`、`sale`、`gatya`、`eventdata`、非Motionの`ut`、`tut`、`st`、更新表示の`skd`、通知設定の`push`、全サーバー共通メンテナー設定とGuild別通知ロール設定の`maint`である。返信とHelpはContent Catalogから起動時に読み、各Commandへ必要な文字列だけを所有させる。Prefix外、未知Command、Guild限定CommandのDM入力ではActionを生成しない。`messageCreate`はCommand Runtimeへ、通知ロールパネルの`reactionAdd`/`reactionRemove`はRolePanelServiceへ、残りの`reactionAdd`と通常Commandの`actionResult`はSession Managerへ振り分ける。Task Runtime経由のDiscord照会結果と通知Actionの結果はそれぞれの待機元へ返す。

RolePanelServiceはGuild設定に保存された単一のmessage IDと1〜9番のReactionだけをRole操作Actionへ変換する。永続パネルにはTTLがないためSession Managerへ登録せず、再起動後もStorage復元だけで動作する。パネル移動後はStorage上のmessage IDを先に新しいMessageへ切り替えるため、旧Messageの無効化に失敗しても旧ReactionからRole操作は発生しない。

## 6. Session経路

Session ManagerはWorkerが所有し、MutexやCommand固有Queueを持たない。CommandがSession付き出力を返すと、最初の`sendMessage` Action IDに待機Sessionを結び付ける。成功した`actionResult`からDiscord message IDを得た後に有効化し、Reaction Actionを返す。

```text
CommandのSession付きsendMessage
→ ActionIdで結果待ち
→ actionResult(messageId)
→ addReaction群
→ 最後のaddReaction結果後にmessageIdで有効化
→ ownerのreactionAdd
→ Session指定のCleanup Action
→ Continuationを1回だけ実行
→ Session削除
```

prompt結果待ち、Reaction準備中、有効中を合わせて最大64件、1 Sessionは最大11 Reaction、TTLは最大5分とする。現在の`sale`は全Reaction追加完了後から30秒、`ut`/`tut`は60秒である。Reaction準備自体は2分を上限とする。Workerは次の最短期限までSleepし、Eventが来ない場合も期限切れ状態を削除する。容量超過時は選択不能な一覧を送らず、再実行案内へ置き換える。

Session継続Actionは元のCommandの`RequestId`を維持する。Action IDは継続を発生させた`actionResult`または`reactionAdd`のEvent IDから作る。`sale`は選択成立時に`clearReactions`を詳細返信より先に実行する。`ut`/`tut`は同じMessageのページ再武装と、候補選択後に別Messageでfile選択を始める継続を利用する。timeout時は全Reactionを削除し、現在の一覧へ受付終了footerを表示する。

## 7. 将来のRuntime構成

必要なPhaseで次を段階的に追加する。

```text
AppRuntime
├─ CommandRuntime      # Phase 5で実装済み
├─ SessionManager
├─ TaskRuntime
├─ ResourceManager
├─ ServiceHub
└─ ActionBus
```

- `CommandRuntime`: Parse、Resolve、Guild判定、Dispatch。詳細Authorizationは必要なCommandで追加する
- `SessionManager`: Phase 9で実装済み。複数Discord Eventを跨ぐ有限・期限付き状態
- `TaskRuntime`: Async I/O、CPU処理、External Processの有限な実行管理
- `ResourceManager`: Queue容量と同時実行Permit
- `ServiceHub`: HTTP、Cache、Storage等の共有基盤。利用者が現れたものだけを追加する

Phase 8で実利用者が生じたため、共有HTTPとClockを追加した。Phase 10では更新頻度と旧仕様が確定している`ut`/`tut` dataだけに、10分TTL、条件付き再検証、検証済みstale fallbackを追加した。Asset存在Cacheは最大512件である。汎用Cache FrameworkとMetricsは追加していない。各機構は必要になるPhaseまで追加せず、Commandごとの独自Queue、Cache、HTTP Client、Timeout基盤は作らない。

## 8. 最終的な処理フロー

```text
Discord
→ TypeScript Adapter
→ CoreEvent
→ AppRuntime
→ Command / Session / Task Runtime
→ CoreAction
→ ActionBus
→ TypeScript Adapter
→ Discord
```

TypeScriptはDiscord APIとの変換だけを担当し、SessionやCommandの意味を持たない。
