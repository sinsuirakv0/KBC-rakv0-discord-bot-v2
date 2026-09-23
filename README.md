# KBC Discord Bot V2

KBC Discord Bot V2は、軽量なDiscord AdapterとRust CoreをKBC Protocolで接続したDiscord Botである。Command、Session、Task、HTTP、StorageなどのBotロジックはRust Coreが所有し、TypeScriptは`discord.js`との変換に限定する。

## Architecture

```text
Discord
  ↓
TypeScript Discord Adapter
  ↓ CoreEvent / CoreAction
KBC Protocol
  ↓ N-API
Rust Core
  ├─ Command / Session / Task Runtime
  ├─ HTTP / Storage / Notification
  └─ Motion render / FFmpeg
```

Rustの`kbc-protocol`がProtocol型の正本であり、TypeScript型はそこから生成する。MotionのFrame dataはN-APIを通さず、Rust所有の一時FileをDiscord Adapterが送信する。

## Requirements

- Node.js 24とnpm
- Rust 1.89以上、Cargo、rustfmt、Clippy
- Discord Bot Token
- Discord Developer Portalで必要なGateway Intentを有効化

FFmpegは`ffmpeg-static`を利用する。別の実行Fileを使う場合だけ`FFMPEG_PATH`を指定する。Windows固有のRust環境については[Workspace実装](docs/implementation/WORKSPACE.md)を参照する。

## Local Development

依存関係を導入する。

```text
npm ci
```

`.env.example`を基に`.env`を作成し、最低限`DISCORD_TOKEN`を設定する。隣接する旧Repositoryの設定を移行する場合は次を使える。

```text
npm run setup:local
```

Release profileのNative moduleとTypeScriptをbuildしてLocal Botを起動する。

```text
npm run local
```

Local送信には`[local] `が付く。詳細は[Local開発起動](docs/implementation/LOCAL_DEVELOPMENT.md)を参照する。

## Build and Validation

```text
npm run build
npm run typecheck
npm run check:rust
cargo fmt --check
cargo clippy --workspace
cargo test --workspace
```

Protocol型を明示的に再生成する場合は`npm run generate:protocol`を使う。CIは生成後の差分も検査し、Rust定義とcommit済みTypeScript型のずれを検出する。

Docker imageはRepository rootで次のようにbuildできる。

```text
docker build -t kbc-discord-bot-v2 .
```

## Environment Variables

| 変数 | 必須 | 用途 |
|---|---|---|
| `DISCORD_TOKEN` | はい | Discord Bot Token |
| `FFMPEG_PATH` | いいえ | `ffmpeg-static`以外のFFmpeg実行File |
| `GITHUB_DATA_OWNER` | 条件付き | 通知設定保存先のGitHub owner |
| `GITHUB_DATA_REPO` | 条件付き | 通知設定保存先のprivate Repository |
| `GITHUB_DATA_BRANCH` | いいえ | 保存先branch。既定は`main` |
| `GITHUB_DATA_TOKEN` | 条件付き | 保存先RepositoryのContents read/write Token |
| `EVENT_UPDATE_SECRET` | いいえ | 外部更新受信を有効化する共有Secret |
| `EVENT_UPDATE_PORT` | いいえ | HTTP受信Port。既定は`PORT`または`3000` |
| `EVENT_UPDATE_HOST` | いいえ | HTTP listen host。既定は`0.0.0.0` |
| `PORT` | いいえ | `EVENT_UPDATE_PORT`未指定時に使うPlatform指定Port |
| `NODE_ENV` | いいえ | `production`ではLocal表示Prefixを無効化 |

GitHub Data関連のowner、Repository、Tokenは3項目を同時に設定する。`EVENT_UPDATE_SECRET`を使う場合はGitHub Data設定も必要である。SecretをRepositoryへcommitしない。

## Directory Structure

```text
apps/discord/        TypeScript Discord Adapter
crates/kbc-protocol/ Protocol型とTypeScript型生成
crates/kbc-core/     Discord非依存のBot Core
crates/kbc-node/     薄いN-API Bridge
content/             起動時に読む返信・Help Content
docs/                Architecture、Decision、実装資料
scripts/             Build・Local開発補助Script
```

## Documentation

- [設計原則](docs/architecture/PRINCIPLES.md)
- [Runtime設計](docs/architecture/RUNTIME.md)
- [KBC Protocol](docs/architecture/PROTOCOL.md)
- [Discord Adapter](docs/architecture/DISCORD_ADAPTER.md)
- [Command追加・変更](docs/engineering/COMMANDS.md)
- [性能・負荷設計](docs/engineering/PERFORMANCE.md)
- [テスト方針](docs/engineering/TESTING.md)
- [Rust Core V2計画](docs/plans/RUST_CORE_V2.md)
