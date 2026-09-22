# V2 Workspace構成

## 構成

```text
apps/
└─ discord/           TypeScript Discord Adapter

crates/
├─ kbc-protocol/      version付きEvent / Action DTO
├─ kbc-core/          Discord非依存Runtime
└─ kbc-node/          薄いN-API Bridge
```

Phase 1ではbuild境界だけを作った。現在はPhase 6まで進み、Protocol、Runtime、Discord Adapter、Command Runtime、Content Catalogを実装済みである。

## Build

```text
npm ci
npm run generate:protocol
npm run build:ts
npm run build:rust
```

全体buildは次を使う。

```text
npm run build
```

Windowsの`gnullvm` toolchainでは、N-API symbolをNode.js processから動的に解決する。`scripts/run-cargo.js`は`napi-build`用の空link shimを`target/libnode/`へ用意し、`scripts/copy-native.js`はNative moduleと`libunwind.dll`を`native/`へ配置する。LinuxとDockerでは追加処理なしでCargoを実行する。

## Docker

DockerはRustとTypeScriptを別stageでbuildする。

1. Rust stageで`kbc-node`をrelease buildする。
2. TypeScript stageでDiscord Adapterをbuildし、production dependencyだけを残す。
3. Runtime imageへJavaScript、production dependency、`kbc_node.node`、`content`をコピーする。

現在のN-API moduleはProtocol情報と`NativeCore`のRuntime APIを公開する。Content rootはN-APIの起動設定として渡す。

## Phase 1検証結果

2026-09-20時点で、次を確認した。

- `npm run build` 成功
- `npm run typecheck` 成功
- `npm run check:rust` 成功
- `cargo fmt --all -- --check` 成功
- `node scripts/run-cargo.js build --release --locked -p kbc-node` 成功
- Docker Hub上に`rust:1.98-bookworm`と`node:24-bookworm-slim`が存在

検証環境はNode.js 24.15.0、npm 11.12.1、Rust 1.98.1である。
このPCにはDocker Engineがないため、実際のImage buildとContainer起動は未実施。Dockerfile内の各build処理はLocalで成功している。

以上により、Phase 1のWorkspace Skeletonは完了とする。実Image buildはDockerを利用できる環境で最初に再確認する。

## 依存ルール

- `apps/discord`へCommand固有Logicを置かない。
- `kbc-node`へDomain Logicを置かない。
- `kbc-core`からNode.js・discord.jsを参照しない。
- `kbc-protocol`からRuntime実装を参照しない。
- crateやpackageは、独立した責務が確認できるまで増やさない。
