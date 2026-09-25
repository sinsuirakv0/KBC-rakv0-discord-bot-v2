# KBC Protocol設計

## 1. 目的

TypeScript Discord AdapterとRust Coreを直接結合せず、明示的なversion付き契約を置く。

```text
discord.js
↓
TypeScript Adapter
↓ CoreEvent / CoreAction
N-API Bridge
↓
Rust Core
```

Rust Coreはdiscord.js Objectを受け取らず、TypeScriptはRust内部Domain Modelを知らない。

## 2. Version 1のEvent

`CoreEvent`は共通Envelopeである。

```text
protocolVersion
eventId
requestId
event
```

現在のEvent Variantは次の4つとする。

- `messageCreate`
- `reactionAdd`
- `reactionRemove`
- `actionResult`

`messageCreate`とReaction Eventには、Discord Objectではなく必要なSnowflakeとplain dataだけを入れる。Discord Snowflakeは精度を失わないよう、すべてStringで保持する。`reactionRemove`は永続的な通知ロール選択パネルでロールを外すために使い、短期Sessionは`reactionAdd`だけを扱う。

`actionResult`はTypeScriptがDiscord操作を終えた結果をCoreへ返すEventである。通常成功時は必要に応じて`messageId`を返し、Guild member解決時は有限な`membersResolved`を返す。失敗時は安定したcodeとretry可否だけを返す。内部例外やStack TraceはProtocolへ載せない。

Button等は必要になるPhaseまで追加しない。ShutdownはEventではなくLifecycle APIとして扱う。

## 3. Version 1のAction

`CoreAction`も共通Envelopeを持つ。

```text
protocolVersion
actionId
requestId
action
```

現行Version 5のAction Variantは次の12個とする。

- `sendMessage`
- `sendNotification`
- `editMessage`
- `sendAttachment`
- `sendAttachmentFile`
- `addReaction`
- `clearReactions`
- `resolveGuildMembers`
- `createGuildRole`
- `resolveAssignableRole`
- `addGuildMemberRole`
- `removeGuildMemberRole`

AttachmentのdataはProtocol上`Uint8Array`とし、N-APIの実運用経路ではNode.js `Buffer`として扱う。base64文字列へ変換しない。

`sendNotification`は`sendMessage`にDiscord nonceと明示的な`allowedRoleIds`を加えた通知専用Actionである。Adapterは`enforceNonce: true`で実行し、同じ通知の応答消失時にDiscord側でも重複を抑止する。Role mentionはCoreが本文と許可IDを決め、Adapterは`allowedMentions.parse`を空にしたまま指定Roleだけを許可する。通常Commandは従来どおり`sendMessage`を使う。

`resolveGuildMembers`は、Coreが渡した最大64件のユーザー・ロールIDと現在のGuild memberをDiscord Adapterで照合する。結果件数はCore指定かつ最大25件とし、全件をProtocolへ流さない。Bot内の権限判断、設定保存、一覧本文の構築はCoreが担当する。

通知ロール操作はDiscord固有のRole Objectと権限階層だけをAdapterで扱う。`createGuildRole`は権限ゼロのRoleを作成し、`resolveAssignableRole`は`@everyone`・managed Role・権限付きRole・Botが管理できないRoleを拒否する。ReactionとRoleの対応、登録上限、通知メンションの設定判断はCoreが担当する。Role解決・作成成功は`roleResolved`でIDと名前を返す。

## 4. ID

Version 1では次を区別する。

- `EventId`: Adapterが受信Eventごとに付与する識別子
- `RequestId`: 一連のCommand処理を追跡する相関ID
- `ActionId`: Discord操作要求と`actionResult`を対応させる識別子

`SessionId`はPhase 9でRust Core内部だけに導入した。Adapterとの相関には既存の`ActionId`、Discord message ID、`RequestId`で十分なため、Protocol fieldは追加しない。`TaskId`もProtocol境界で必要になるまで追加せず、全Variantへ将来用nullable fieldを置かない。

1つのEventから複数Actionを生成する場合、先頭は`action:<eventId>`、以降は`action:<eventId>:2`、`action:<eventId>:3`のようにEvent内の順序を付ける。Protocol fieldやVersionは変更せず、各Actionは同じ`RequestId`を保持する。

Sessionが複数Eventを跨いでActionを生成する場合も、Action IDは直前のEvent IDから作り、Request IDはSessionを開始したCommandの値を維持する。

## 5. Rust型とTypeScript型

Rustの`kbc-protocol`を唯一の型定義源とする。

```text
Rust struct / tagged enum
↓ ts-rs
generated TypeScript types
↓
Discord Adapter
```

Wire fieldはcamelCase、Variant識別子は`type`とする。生成されたTypeScriptファイルは手動編集しない。

`kbc-protocol`と`kbc-core`はN-APIへ依存しない。JavaScript valueとの変換は`kbc-node`だけが担当する。

## 6. Version互換性

現在の`PROTOCOL_VERSION`は`5`である。Version 5では`reactionRemove`、通知Role操作Action、`roleResolved`、`sendNotification.allowedRoleIds`を追加したためVersionを上げた。

AdapterはNative module読込時に`getRuntimeInfo()`を呼び、Rust CoreとAdapterのVersionが一致しなければ起動を失敗させる。

既存fieldの意味変更、必須field削除、既存Variantの互換性を壊す変更ではProtocol Versionを上げる。互換的な追加であっても、Adapterが未知Variantを処理できない場合は同様にVersionを上げる。

## 7. N-API公開面

Phase 3の公開面は次に限定する。

```text
getRuntimeInfo()
createCore(config?)
└─ NativeCore
   ├─ submitEvent(event)
   ├─ nextAction()
   └─ shutdown()
```

`createCore()`は任意のEvent Queue容量とAction Queue容量を受け取り、`NativeCore`を返す。`submitEvent()`と`nextAction()`は非同期であり、bounded Queueのbackpressureをそのままcallerへ伝える。停止後の`nextAction()`は`null`を返す。

Phase 2限定の`roundTripEvent()`は削除済みである。Protocol境界は実際のRuntime経路を通るSmokeで検証する。CommandごとのN-API関数は追加しない。

外部更新通知はDiscord Eventではないため`CoreEvent`へ混在させない。通知機能では外部入力境界として`prepareNotifications()`と`submitDetection(value)`を追加した。TypeScriptはHTTP形式とSecretを検査し、Detection EventのDomain検証と配送判断はRust Coreが行う。
