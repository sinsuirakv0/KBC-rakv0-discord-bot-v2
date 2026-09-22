# Local開発起動

## 初回セットアップ

V2の`.env`を作成または更新する場合は、次を実行する。

```text
npm run setup:local
```

既定では隣接する旧Repositoryの`.env`を読み、`DISCORD_TOKEN`をV2の`.env`へコピーする。通知用の値がある場合は、`WORKFLOW_GITHUB_TOKEN`または`GITHUB_DATA_TOKEN`、`EVENT_UPDATE_SECRET`、Portも引き継ぐ。GitHub保存先は`sinsuirakv0/KBC-rakv0-discord-bot-data`を既定とする。既存のV2設定を優先し、Token値は出力しない。別の移植元を使う場合は`KBC_LOCAL_SOURCE_ENV_FILE`で指定する。

`.env`は`.gitignore`の対象であり、Repositoryへ追加しない。設定例は`.env.example`に置く。

## Buildして起動

通常は次の1 Commandを使う。MotionなどCPU負荷の高い処理を本環境に近い速度で確認できるよう、Rust Native moduleはRelease profileでbuildする。

```text
npm run local
```

Rust Native moduleをRelease profileで、TypeScriptを通常どおりbuildした後、BotをLocalモードで起動する。送信MessageにはDiscord Adapterが`[local] `を付ける。初回Release buildはDebug buildより時間がかかるが、以後は差分buildになる。

## Build済みの高速起動

CodeとNative moduleをbuild済みなら次を使う。

```text
npm run start:local
```

`start:local`はV2の`.env`を優先する。V2の`.env`がなければ、互換のため隣接する旧Repositoryの`.env`を読む。明示的なFileを使う場合は`KBC_LOCAL_ENV_FILE`で指定する。

Local起動Scriptは子Processの`NODE_ENV`を必ず`development`にし、本番Messageと区別する。

## 停止

Terminalで`Ctrl+C`を送る。Discord Adapterが`SIGINT`を受け、Core shutdown完了後に停止する。

## 検証結果

2026-09-21に次を確認した。

- `setup:local`がToken値を表示せず、必要なLocal設定をV2 `.env`へ作成した。
- `npm run local`だけでbuildとDiscord loginが成功した。
- Local返信に`[local] `が付いた。
- `Ctrl+C`で`Discord adapter stopped.`まで到達し、Bot Processが残らなかった。
