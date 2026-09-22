# Protocol Version 1の初期契約

- 状態: 採用
- 決定日: 2026-09-20

## 決定

Version 1を次の最小契約で開始する。

- Event: `messageCreate`、`reactionAdd`、`actionResult`
- Action: `sendMessage`、`editMessage`、`sendAttachment`、`addReaction`、`clearReactions`
- ID: `EventId`、`RequestId`、`ActionId`
- Wire形式: camelCase fieldを持つSerde tagged enum
- 型定義源: Rust `kbc-protocol`
- TypeScript型: `ts-rs`による生成物

EventとActionは共通Envelopeを持ち、Variant固有dataをtagged enumとして表す。SessionとTaskのIDは該当Runtimeを実装するまで追加しない。

## 理由

現行機能の移植に必要なDiscord入出力を表現でき、`actionResult`によって送信後の`messageId`や失敗をCoreへ返せる。Rust enumによりVariantごとの必須fieldを固定し、将来用nullable fieldの集合を避けられる。

RustからTypeScript型を生成するため、同じProtocolを手動で二重定義しない。N-API変換は`kbc-node`へ限定し、Protocol型自体はNode.js非依存を維持する。

## トレードオフ

- 初期VariantにないButtonやTask cancelは、必要時にProtocol変更が必要になる。
- TypeScript生成を先に実行しないとTypeScript buildが成立しないため、npm scriptへ生成工程を含める。
- Attachmentの`Uint8Array`とNode.js `Buffer`の変換・size上限は、実際のAction経路を作るPhaseで検証が必要になる。

## 変更条件

新しいDiscord入力・出力が既存Variantで自然に表現できない場合にだけVariantを追加する。既存fieldの意味を流用しない。互換性を壊す場合は`PROTOCOL_VERSION`を上げ、AdapterとCoreを同時に更新する。
