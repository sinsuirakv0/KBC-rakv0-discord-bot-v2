# Phase 10以降 作業チェックポイント

- 状態: 非Motion Command移植完了、V2本環境稼働開始
- 最終更新: 2026-09-22
- 対象: 非Motion Command、Storage、Notification Runtime

## 確定した順序

1. Motion以外のCommandをすべてV2へ移植する。
2. V2を本環境で動かし、通常Commandと通知を観測する。
3. 本環境の計測結果を使ってMotionを移植・改善・高速化する。

`utbench`は旧版で必要な計測結果を取得済みのため、V2へ移植しない。Motion本体とTask Runtimeも本環境稼働後まで保留する。

## 完了したCommand

- 静的Command: `ping`、`home`、`asset`、`skb`、`skdsite`
- 外部Data Command: `gatya`、`sale`、`item`、`eventdata`
- 検索Command: 非Motionの`ut`、`tut`、`st`
- 更新表示: `skd`
- 通知設定: `push`

`ut`、`tut`、`st`、`sale`のReaction Sessionは、選択後とtimeout後に全Reactionを削除する。

## Storage・通知

- private GitHub Repositoryを正本とする`StorageService`をRust Coreへ追加した。
- `o.push skd/ad/notice`と末尾`off`を実装した。
- 固定Bot管理者3人だけが現在の設定変更対象である。健康維持メンテナーロール設定は従来どおり後続作業とする。
- 通知予定をDiscord送信前に保存し、結果不明の`attempting`は自動再投稿しない。
- `detected`、`types`、`ready`の段階更新、同じEvent IDの重複防止、本文編集、SKD詳細投稿をRust Coreへ実装した。
- TypeScriptは認証付きHTTP受信とDiscord Action実行だけを担当する。
- nonce付き`sendNotification`追加に伴いKBC ProtocolをVersion 2へ更新した。
- 詳細は`docs/decisions/NOTIFICATION_RUNTIME_V1.md`を参照する。

## 検証結果

- `st`: landing、ID検索、文字検索、一覧、Reactionページ操作を実Dataで確認した。
- `eventdata`: 公式URL、KBC URL、平文File、暗号化Fileを実Dataで確認した。
- `skd`: Git履歴から最新更新とgatya/sale/item/KBCリンクを実Dataで確認した。
- `push`: Storage未設定時の案内、権限判定、実Discordからの登録、既存登録と同じ内容の冪等な再登録を確認した。本環境とのSHA競合時は最新版を再読込する1回の再試行で成功した。
- GitHub Storage: private repository、branch、`meta.json`、Guild設定復元を確認した。
- Webhook: live health 200、認証なし401、不正Event 400を確認した。
- Local Bot: Discord login、通知health 200 ready、SIGINT正常停止を確認した。
- Docker: Linux release image buildとContainer内のRust Native Runtime Smokeが成功した。
- Northflank: `sinsuirakv0/KBC-rakv0-discord-bot-v2`の`976c7c3`をDockerfileからbuildし、既存Serviceへ手動deployした。旧Container終了後にV2が起動し、1/1 Runningを確認した。
- 本環境: Discord login、`/health/live` 200 alive、`/health` 200 ready、GitHub Storageと通知受信設定の復元を確認した。
- Rustfmt、Clippy警告ゼロ、Rust test 9件、TypeScript typecheck、Native/TypeScript buildが成功した。

## 本環境移行後に残る作業

1. 実際の更新検知Eventで、初回投稿、種類追記、SKD詳細、KBCリンク、永続Checkpointを確認する。
2. 本環境の通常Commandを利用者側から確認する。
3. 本環境の計測結果を取りながらMotion設計・移植・改善・高速化へ進む。

実在しない有効EventによるE2E試験は、永続通知履歴とDiscord Channelを汚すため実施していない。最初の実更新をE2E確認に使う。

## 再開位置

Northflankの既存ServiceはV2 Repositoryへ切り替え済みで、CI/CDも再有効化した。次回は本環境の通常Commandと最初の実更新通知を観測しつつ、Motion設計へ進む。

`D:\KBC\KBC-rakv0-discord-bot`は読み取りだけに使用し、変更していない。
