# Discord Adapter設計

## 1. 責務

TypeScript Discord Adapterは、DiscordとKBC Protocolの相互変換だけを担当する。

```text
Discord Gateway Event
→ CoreEvent
→ NativeCore

NativeCore
→ CoreAction
→ Discord API
→ actionResult CoreEvent
```

Command解析、Session状態、再試行Policy、Domain LogicをTypeScriptへ置かない。

## 2. Gateway Event

初期Adapterは次を受け取る。

- `messageCreate` → `CoreEvent.messageCreate`
- `messageReactionAdd` → `CoreEvent.reactionAdd`
- `messageReactionRemove` → `CoreEvent.reactionRemove`

Bot自身を含むBot userのEventは除外する。Discord SnowflakeはStringのまま渡し、Discord Object自体はCoreへ渡さない。

Unicode Reactionは`discord.js`の`identifier`ではURL encodeされるため、Protocolへは生のemoji `name`を渡す。Custom emojiはGuild内で一意に判定できる`identifier`を維持する。Adapterは選択状態を持たず、この表現変換だけを担当する。

ClientはGuild、Guild Members、Guild Message、Message Content、Guild ReactionのIntentを使う。Guild Membersは`o.maint maintainer list`でロール所属者を漏れなく解決するために必要であり、Developer Portal側でもServer Members Intentを有効にする。cacheにないMessageへのReactionもIDとして受け取れるよう、Message、Channel、Reactionのpartialを有効にする。DM用Intentは追加しない。

Gateway Eventごとに新しい`eventId`と`requestId`を生成する。Action実行結果では、対象Actionの`requestId`を維持して一連の処理を追跡可能にする。

## 3. GatewayとBackpressure

Discord.jsのEvent listenerはCoreのQueue空きをawaitしてGatewayへbackpressureを返せない。この差を吸収するため、Adapter全体で1つのbounded FIFOを持ち、Eventを登録順に`submitEvent()`へ渡す。

- 容量: 64件
- drain数: 1
- 満杯時: silent dropせずfatal errorとしてAdapterを停止
- shutdown時: 新規受付を止め、未送信Eventを破棄

CommandごとのQueueは作らない。容量変更は実運用のqueue waitとoverflowを計測してから判断する。

Dispatcherは現在のoutstanding数、起動後のhigh-water mark、overflow回数を保持する。通常時はhigh-water markが更新された時だけ、overflow時は毎回、次の形式で記録する。Eventの種類別dropや優先制御は行わず、capacity 64到達時のfatal shutdown方針は維持する。

```text
Event dispatcher metrics: outstanding=4 high_water=4 overflow_count=0
```

## 4. Core Action

ActionはAdapter内の有限Dispatcherで実行する。

- 全体同時実行数: 3
- 実行中と待機中の合計: 最大16件
- Channelを持つAction: `channelId`ごとにFIFO
- `resolveGuildMembers`とRole操作: `guildId`ごとにFIFO

同一Channelでは、複数返信の表示順、`clearReactions`後の編集・返信、通知の送信後編集を維持する必要があるため直列実行する。Session、Task Runtime、Notification ServiceがDiscord操作の結果に依存する後続Actionは、先行Actionの`actionResult`をCoreが受信してから生成される。したがって`ActionId`の対応を維持したまま、異なるChannelのActionは並行実行できる。

Dispatcherは待機中の先頭だけを機械的に実行せず、現在実行中ではないordering keyの最古Actionを選ぶ。同一Channelの待機Actionが実行枠を占有して、別Channelを再び停止させないためである。上限到達時は`nextAction()`の取得を止め、Coreのbounded Action Queueへbackpressureを返す。

| Core Action | Discord操作 | 成功時messageId |
|---|---|---|
| `sendMessage` | ChannelへMessage送信 | 送信Message ID |
| `sendNotification` | nonceを強制してChannelへ通知送信 | 送信Message ID |
| `editMessage` | Message編集 | なし |
| `sendAttachment` | Bufferを添付して送信 | 送信Message ID |
| `sendAttachmentFile` | Task所有のFileを添付して送信 | 送信Message ID |
| `addReaction` | MessageへReaction追加 | なし |
| `clearReactions` | MessageのReactionを全削除 | なし |
| `resolveGuildMembers` | 指定ユーザー・ロールIDに一致するGuild memberを有限件解決 | `membersResolved` |
| `createGuildRole` | 権限ゼロの通知用Roleを作成 | `roleResolved` |
| `resolveAssignableRole` | Roleが権限ゼロかつBot管理可能か確認 | `roleResolved` |
| `addGuildMemberRole` | 選択Reactionに対応するRoleをMemberへ付与 | `success` |
| `removeGuildMemberRole` | 選択Reactionに対応するRoleをMemberから解除 | `success` |

成功・失敗にかかわらず、元の`ActionId`と`RequestId`を維持した結果を`actionResult`としてCoreへ返す。異なるordering keyの結果順は実行完了順となるが、Session Manager、Task Runtime、Notification Serviceは`ActionId`で待機元を特定する。Adapterは自動再試行せず、失敗を安定したcodeとretry可否へ分類する。内部ErrorやStack TraceはProtocolへ含めず、Local logだけへ出す。

Message送信と編集では`allowedMentions.parse`を空にする。通常の`sendMessage`を使う`o.skd`を含め、Coreからの文字列だけでUser、Role、全員へmentionしない。更新通知の`sendNotification`だけはProtocolの`allowedRoleIds`に列挙されたRoleを明示許可し、その他のmentionは無効のままとする。

Local実行では送信・編集本文の先頭へ`[local] `を付け、本番Botと区別する。`NODE_ENV=production`の場合だけprefixを付けない。本文なしのAttachmentをLocalから送る場合は`[local]`自体を本文とする。Docker runtimeは`NODE_ENV=production`を明示する。

## 5. 外部更新受信

`EVENT_UPDATE_SECRET`が設定された場合、AdapterはHTTP受信Serverを起動する。

- `GET /health/live`: Process生存確認
- `GET /health`: Discord接続とStorage準備完了の確認
- `POST /event-update`: Secret、JSON Content-Type、16 KiB上限を検査してCoreへ渡す

HTTP Serverは通知の意味を解釈しない。Coreの結果をaccepted、invalid、reconciliation-required、busy、delivery-failedへ写像する。Storageが一時的に利用できない場合は30秒間隔で準備を再試行する。

## 6. Lifecycle

起動順は次とする。

```text
Config読込
→ NativeCore作成
→ Discord Client作成
→ Event listenerとAction loop開始
→ Discord login
```

`SIGINT`、`SIGTERM`、Adapter内部のfatal errorで同じ冪等shutdownを使う。

```text
新規Gateway Event停止
→ Discord Client破棄
→ Action Dispatcherの新規受付停止・待機Action破棄とAdapter Event FIFO停止
→ NativeCore shutdown
→ Action loopと実行中Actionの終了待ち
```

## 7. Command経路

Phase 5以降、AdapterはCommand Prefixを判定せず、Bot以外の`messageCreate`をすべてCoreへ渡す。CoreのCommandRuntimeが`o.` Prefix、Command名、Guild条件を判断する。現在の実Discord疎通には`o.ping`を使う。
