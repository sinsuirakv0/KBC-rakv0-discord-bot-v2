# Bot status仕様

## 目的

`o.botstatus`から、Botの現在状態、Northflank上の実行環境、直近の負荷、稼働時間を1画面で確認できるようにする。Slash Commandは使用しない。

## 表示内容

- ServiceとContainerの稼働状態
- 現在Containerの稼働時間
- V2初回Deploymentからの累計稼働時間
- CPU・Memoryの現在値、平均値、最大値
- Network ingress・egress、Request、HTTP 5xxの直近値
- Region、Plan、Instance数、OS、Architecture
- Branch、Commit、Build・Deployment状態、Rust Core・Node.js Version
- CPUとMemoryの時系列Graph

GraphはPNGを生成し、Discord Messageの添付画像をEmbed内へ表示する。外部の画像保存先は使用しない。

## 操作

- `30分`と`1時間`で表示範囲を切り替える。
- `今すぐ更新`で即時取得し、自動更新を再開する。
- 実行中は15秒ごとに自動更新する。
- 最後にボタンを押してから1分で自動更新を停止する。
- `停止`で直ちに自動更新を停止する。
- 停止後15分までは`更新を再開`でき、その後はButtonを無効化する。
- ButtonはCommand実行者だけが操作できる。

同時に保持する画面はBot全体で最大8件とする。同じ期間のNorthflank応答は画面間で共有し、画面数に比例したAPI呼出しを発生させない。

## 稼働時間

現在の稼働時間は、実行中Containerの作成時刻から算出する。

累計稼働時間は`state/bot-uptime.json`へ保存する。初回だけV2初回Commit `976c7c3`のDeployment時刻から機能初期化時刻までを初期値にし、以降はBot processが実際に動作した時間だけを加算する。再デプロイで旧・新processが重なる場合は保存時に時刻区間を結合し、重複加算しない。

初回Deployment履歴を取得できない場合はCommit時刻を代替値として推定表示する。正確な時刻を既知の場合は`BOT_UPTIME_STARTED_AT`で上書きできる。GitHub Storageが未設定の場合も表示は行うが、累計値が永続化されないことを明記する。

## 設定

- `NORTHFLANK_API_TOKEN`
- `NORTHFLANK_PROJECT_ID`（Northflank上では`NF_PROJECT_ID`を自動利用）
- `NORTHFLANK_SERVICE_ID`（Northflank上では`NF_OBJECT_ID`を自動利用）
- `BOT_UPTIME_STARTED_AT`（任意、ISO 8601）

Token、Project ID、Service IDが揃わない場合は機能を無効にし、Bot本体の起動は継続する。累計値の永続化には既存のGitHub Storage設定を使用する。

## 実環境確認

- 初回表示と15秒更新
- 30分・1時間の切替とGraph差替え
- 1分無操作停止、手動停止、再開、15分後の無効化
- 実行者以外のButton操作拒否
- NorthflankのCPU・Memory・Container・Deployment値との一致
- 再起動前後の累計稼働時間と`state/bot-uptime.json`
- Northflank API障害・権限不足時の安全なエラー表示
