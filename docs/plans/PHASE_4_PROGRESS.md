# Phase 4 作業チェックポイント

- 状態: 完了
- 最終更新: 2026-09-21
- 対象: Discord Adapter

## 調査結果

- 旧実装はGuild、Guild Message、Message Content、Guild ReactionのIntentを使う。
- 常駐listenerは`messageCreate`だけで、ReactionはCommand内collectorが直接待つ。
- Discord操作はsend、edit、attachment、reaction追加、reaction全削除を使う。
- 旧実装にはDiscord ClientとCoreをまとめるshutdown lifecycleがない。

## 確定した方針

- TypeScriptはDiscord EventとCoreEvent、CoreActionとDiscord APIの変換だけを担当する。
- Gateway Eventは容量64件の共有bounded FIFOで順序を保ってCoreへ渡す。
- FIFO overflowはsilent dropせずfatal停止とする。
- Core Actionは直列実行し、結果を必ず`actionResult`として返す。
- Messageのmention解析は既定で無効にする。
- Localの送信・編集本文は`[local] `で始め、Docker本番では`NODE_ENV=production`により付与しない。
- `SIGINT`、`SIGTERM`、内部fatal errorで共通の冪等shutdownを使う。
- Phase 4の実Discord疎通は完全一致の`o.runtime-smoke`だけに限定する。

## 作業状況

- [x] 関連資料と旧Discord境界の読み取り調査
- [x] Adapter境界とbackpressure方針の確定
- [x] Architecture Decision作成
- [x] Event変換とbounded FIFO実装
- [x] Action実行とAction Result実装
- [x] Discord Client lifecycle実装
- [x] Dummy Coreの安全な疎通条件追加
- [x] Compiler・Clippy・format・Smoke検証
- [x] 実装Document更新
- [x] 旧`.env`のTokenをFileへ複製せず一時利用
- [x] 実Discord login確認
- [x] `o.runtime-smoke`の送受信確認
- [x] Signal shutdown確認

## 完了状況

Adapter実装、DebugとRelease build、Runtime Smoke、module load、Compiler、format、Clippy、実Discord疎通まで成功した。旧リポジトリの`.env`からTokenをProcessへだけ渡し、TokenはV2のFileやDocumentへ複製していない。

実Discordでは次を確認した。

- 「超健康bot Hugin#9102」としてlogin成功
- `o.runtime-smoke`に対して`[local] o.runtime-smoke`が同じChannelへ返る
- Action実行後にfatal errorがない
- `SIGINT`受付後に`Discord adapter stopped.`まで完了する

## 次の開始位置

次はPhase 5 — CommandRuntimeの設計から開始する。Dummyの`o.runtime-smoke`分岐は、最初の単純CommandをCommandRuntimeへ載せた時点で削除する。
