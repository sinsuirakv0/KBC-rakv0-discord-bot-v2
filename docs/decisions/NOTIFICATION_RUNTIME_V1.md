# Notification Runtime V1

## 決定

`push`は通知先設定だけで完了とせず、外部更新の受信、GitHubへの配送Checkpoint保存、Discord配送結果の反映までRust Coreで扱う。

TypeScriptは次だけを担当する。

- 認証付きHTTP `/event-update`、`/health`、`/health/live`
- 16 KiBのBody上限とHTTP形式の検査
- JSONをRust Coreの`submitDetection`へ渡す
- Rust Coreが発行したDiscord Actionの実行と`actionResult`の返却

## 境界

外部更新はDiscord Eventではないため、KBC Protocolの`CoreEvent`へ混在させない。N-APIに外部入力用の`submitDetection`と起動確認用の`prepareNotifications`を追加する。入力のDomain検証、重複防止、配送判断はRust側で行う。

通知の初回投稿はDiscord nonceを必須にする。既存`sendMessage`の意味を変えず、`sendNotification` Actionを追加するためProtocol Versionを2へ上げる。Adapterは`nonce`と`enforceNonce: true`をそのままDiscordへ渡す。

## 有限性

- 外部更新は最大4件まで受付し、超過時はbusyを返す。
- 配送処理は全体で1件ずつ直列化し、同じ更新の競合を避ける。
- Discord Action結果待ちは30秒で打ち切る。
- GitHub Storage、HTTP取得、Discord Action Queueは既存の共有限界を使う。
- Command固有のHTTP Client、無制限Queue、無制限Taskは作らない。

## 永続化順序

1. 更新Eventと対象通知先をGitHubへ保存する。
2. 各投稿を`attempting`として保存する。
3. `sendNotification`または`editMessage`を発行する。
4. `actionResult`成功後にmessage IDと本文を`sent`として保存する。

送信後に結果を確認できなかった`attempting`は自動再投稿しない。次回の同一Event受信では`reconciliation-required`を返す。編集失敗はmessage IDが確定しているため、次回に同じ投稿を再編集できる。

## SKD詳細

`ready` Eventでは指定されたbefore/after Git revisionとTSV hashを検証し、`o.skd`と同じParser・差分・Formatterを再利用する。生成した詳細本文をGitHubへ一度保存してから、各通知先へ同じ順序で配送する。

## 代替案を採らない理由

- TypeScriptへ旧Notification Serviceをそのまま移す案は、Discord非依存の状態機械とStorage判断がAdapterへ残る。
- HTTP受信をRustへ内蔵する案は、Network ListenerのLifecycleまでCoreに持ち込み、NorthflankのPORTやNode process終了処理との境界が複雑になる。
- 受信直後に202だけ返す案は、従来の503・409による再実行判断を失う。
