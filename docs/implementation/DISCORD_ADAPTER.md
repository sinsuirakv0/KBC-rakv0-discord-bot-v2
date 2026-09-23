# Discord Adapter実装

## 実装範囲

Phase 4では、Discord Gateway EventをCoreEventへ変換し、CoreActionをDiscord APIで実行してAction ResultをCoreへ返すTypeScript Adapterを実装した。Command解析やSession状態は持たない。

## 関数と相互関係

### Config

- `loadDiscordAdapterConfig(environment)`: `DISCORD_TOKEN`を検証し、Localでは`[local] `、`NODE_ENV=production`では空の送信prefixを返す。Token値はlogへ出さない

### Event変換

- `createMessageCreateEvent(message)`: MessageのID、Role ID、本文だけを`messageCreate`へ変換する
- `createReactionAddEvent(reaction, user)`: partialを含むReactionから必要なIDとemoji identifierだけを`reactionAdd`へ変換する
- `createActionResultEvent(action, outcome)`: Action IDと元のRequest IDを維持した`actionResult`を作る
- `createEvent(event, requestId?)`: UUIDを使ってEvent envelopeを作る

### CoreEventDispatcher

- `submit(event)`: 容量64件のFIFOへ同期的に枠を確保し、登録順に`NativeCore.submitEvent()`へ渡す
- `drain()`: 1つだけ動作し、同時submitによるEvent順序変更を防ぐ
- `close(reason?)`: 新規Eventを拒否し、未送信EventのPromiseを終了させる

容量にはCoreへ送信中の1件も含める。Core送信失敗または容量超過はAdapter全体のfatal errorとして扱う。

### Action実行

- `executeCoreAction(client, action, outgoingMessagePrefix)`: Protocol Actionをdiscord.js操作へ変換し、`success`、`membersResolved`のいずれかを返す。送信・編集本文へ環境別prefixを付ける
- `resolveGuildMembers`: 最大64件の登録IDと最大5,000人のGuild memberを照合し、名前順で最大25人を返す。完全なRole所属者取得にはServer Members Intentを使用する
- `createAttachmentMessage(prefix, message)`: 本文なしのLocal Attachmentにも`[local]`識別子を付ける
- `getSendableChannel(client, channelId)`: fetchしたChannelがtext basedかつsend可能であることを確認する
- `getMessage(client, channelId, messageId)`: edit以外のMessage対象操作に使うMessageを取得する
- `createActionFailure(error)`: 内部Errorを安定したcodeとretry可否へ変換する

Discord requestのHTTP statusが429または5xxの場合と、statusを取得できないtransport errorをretry可能とする。Adapter自身は再試行しない。

### DiscordAdapter

- `DiscordAdapter.create(config, callbacks)`: Native Coreと必要最小限のIntent・partialを持つClientを作る
- `start()`: listenerとAction loopを開始してDiscordへloginする
- `dispatchEvent(event)`: Gateway handlerから共有FIFOへEventを渡す
- `consumeActions()`: Actionを直列実行し、結果を同じFIFOからCoreへ戻す
- `fail(error)`: fatal errorを通知し、共通shutdownを開始する
- `shutdown()`: 冪等な停止Promiseを返す
- `performShutdown()`: listener、Client、FIFO、Core、Action loopを順に停止する

`src/index.ts`は直接実行された場合だけConfigを読み込んで起動する。Libraryとしてimportした場合はDiscord接続を開始しない。`SIGINT`と`SIGTERM`は同じ`shutdown()`へ接続する。

Docker runtimeは`NODE_ENV=production`を設定する。Localの既定値は`[local] `であり、環境変数が未設定でも本番Botと見分けられる。

## Action Result

```text
CoreAction
→ executeCoreAction
├─ success(messageId?)
├─ membersResolved(members, truncated)
└─ failure(code, retryable)
→ createActionResultEvent
→ CoreEventDispatcher
→ NativeCore.submitEvent
```

Discord APIのError詳細はLocal logへ残すが、CoreへはStack Traceや生のResponseを渡さない。

## Attachment

`sendAttachment`はProtocolのdataを`Buffer.from()`でDiscord uploadへ渡す。現在の`ping` CommandはAttachmentを生成しないため、最初のAttachment対応Commandを実装する前に次を確認する。

- N-API実運用経路の`Uint8Array` / Buffer表現とcopy回数
- 最大Attachment size
- `contentType`の扱い。現在のdiscord.js message uploadはBufferからMIMEを明示指定できないため、file nameからの推定になる

この確認のためだけに恒久的なCommandやTestを追加しない。

## Phase 4検証

2026-09-21時点で次を確認した。

- TypeScript typecheck成功
- Rust workspace check成功
- Rust format check成功
- 全targetのClippyをwarning禁止で実行し成功
- Adapterを含むDebug build成功
- `o.runtime-smoke`によるRuntime Smoke成功
- Windows release Native moduleのbuildとRuntime Smoke成功
- build後のAdapter moduleをimportしても自動接続しない
- 旧リポジトリのTokenをProcessへだけ渡して実Discord login成功
- `o.runtime-smoke`に対して`[local] o.runtime-smoke`が返信されることを確認
- Action実行後のlogにfatal errorがないことを確認
- `SIGINT`受付後に`Discord adapter stopped.`まで完了することを確認

TokenはV2のFile、Document、command line、logへ複製していない。以上によりPhase 4を完了とする。
