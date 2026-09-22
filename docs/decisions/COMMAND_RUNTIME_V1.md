# Command Runtime V1の最小API

- 状態: 採用
- 決定日: 2026-09-21

## 決定

Phase 5のCommand Runtimeを次の責務に分ける。

```text
CommandRuntime
├─ Parser
├─ CommandRegistry
├─ Dispatcher
├─ CommandContext
└─ Command trait
```

- Parserは固定Prefix `o.`を検査し、Command名をASCII lowercaseへ正規化して引数を空白単位で保持する。
- RegistryはCommand名とaliasを同じCommandへ解決し、重複登録を起動時errorとする。
- Metadataは`name`、`aliases`、`guildOnly`だけを持つ。
- Command APIはboxed Futureを返し、将来の非同期ServiceをCommandごとの専用Runtimeなしで利用可能にする。
- Phase 5のCommand出力は`Option<CoreActionData>`とし、1回の実行につき最大1 Actionへ限定する。
- Action envelope、Action ID、Request IDはCommandではなくCommandRuntimeが作る。

最初のCommandとして、Guild限定の`ping`を登録する。引数を無視し、`sendMessage("pong!!")`を返す。

## 理由

Parser、Registry、Discord非依存Context、Action生成の境界を最初の単純Commandで確定できる。CommandはProtocolのAction dataだけを返すため、discord.js、N-API、Queueを知らない。

Futureを返すtraitにより、後続CommandがHTTP等をawaitしてもCommand APIを全面変更せずに済む。一方、Phase 5で複数Action用BufferやEmitterを先に作らないことで、必要性のないFramework化を避ける。

## トレードオフ

- boxed FutureにCommand実行ごとの小さなallocationが発生する。Command処理やDiscord I/Oに比べて十分小さく、計測前には最適化しない。
- 1実行1 Actionのため、複数MessageやProgressはまだ表現できない。
- CommandContextは現在必要なChannel IDだけを持ち、User、Role、Service等は該当Commandの導入時に追加する。
- PrefixはConfiguration Service導入前のため固定値とする。

## 変更条件

最初の複数Action Commandを実装する前に、Action Queueへ直接backpressureを伝えるbounded emitterを検討する。単純な無制限`Vec<CoreActionData>`へは変更しない。

AuthorizationやServiceが必要になった場合は、CommandContext全体をGlobal Service Locatorにせず、必要なCapabilityだけを追加する。
