# Protocol実装

## 型生成

`crates/kbc-protocol/src/lib.rs`がProtocolの定義源である。次のCommandがTypeScript型を`apps/discord/src/protocol/generated/`へ生成する。

```text
npm run generate:protocol
```

`generate_types`は`CoreEvent`、`CoreAction`、`RuntimeInfo`と依存型を`ts-rs`で出力し、同じRust定数から`version.ts`を生成する。生成ファイルは手動編集しない。

`npm run build:ts`と`npm run typecheck`は、TypeScript Compilerの前にこの生成処理を実行する。DockerではRepository内の生成済み型を使って`npm run compile:ts`を実行する。

## 関数と相互関係

### Rust Protocol

- `validate_protocol_version(actual)`: `PROTOCOL_VERSION`との一致を確認する共通関数
- `CoreEvent::validate_version()`: EventのVersionを共通関数へ渡す
- `CoreAction::validate_version()`: ActionのVersionを共通関数へ渡す
- `resolveGuildMembers` / `membersResolved`: Core内のメンテナー設定を、Discord Adapterで現在のGuild memberへ有限件解決する

### N-API Bridge

- `get_runtime_info()`: Rust側のProtocol VersionとCore VersionをJavaScriptへ返す
- `create_core(config)`: Queue設定から`AppRuntime`を開始し、`NativeCore`を返す
- `NativeCore::submit_event(value)`: JavaScript Objectを`CoreEvent`へ変換してRuntimeへ渡す
- `NativeCore::next_action()`: Runtimeの`CoreAction`をJavaScript Objectへ変換する。停止後は`null`を返す
- `NativeCore::shutdown()`: Runtimeへ停止を通知し、Worker終了を待つ
- `protocol_input_error(reason)`: 入力不正をN-API `InvalidArg`へ変換する
- `runtime_error(error)`: 設定・Protocol不正を`InvalidArg`、Lifecycle異常を`GenericFailure`へ変換する
- `bridge_error(error)`: RustからJavaScriptへの変換失敗をN-API errorへ変換する

Phase 2限定の`round_trip_event`は削除した。`kbc-node`にはCommandやDomain Logicを置かない。

### TypeScript

- `createCore(config?, nativePath?)`: `.node`を読み込み、`getRuntimeInfo()`でVersion互換性を検査してからNative Coreを作る
- `copy-native.js`: platform別Native libraryを`native/kbc_node.node`へ配置する。Windows `gnullvm`ではruntime dependencyの`libunwind.dll`も配置する
- `run-cargo.js`: Windows `gnullvm`で動的N-API symbol解決を使えるよう、`napi-build`が要求する空のlink shimを`target/libnode/`へ用意してCargoを実行する

Windows用link shimにはN-API実装を含めない。実行時のN-API symbolはNode.js processから動的に解決する。

## Smoke

```text
npm run smoke:protocol
```

`o.PING ignored`を持つ1つの`messageCreate`をTypeScriptからNative moduleへ渡す。

```text
TypeScript CoreEvent
→ N-API
→ Serde
→ Rust CoreEvent enum
→ Version検査
→ AppRuntime / CommandRuntime / ActionBus
→ Rust CoreAction enum
→ Serde / N-API
→ TypeScript CoreAction
```

同じChannelと`pong!!`を持つ`sendMessage`が返ること、および`shutdown()`が待機中の`nextAction()`を`null`で終了させることだけを確認する。単純な変換ごとのUnit Testは追加しない。

`smoke:protocol`は互換名として`smoke:runtime`を呼ぶ。

## Phase 2検証結果

2026-09-20時点で、次を確認した。

- `npm run smoke:protocol` 成功
- `npm run typecheck` 成功
- `npm run check:rust` 成功
- `cargo clippy --workspace --all-targets -- -D warnings` 成功
- `cargo fmt --all -- --check` 成功
- Windows release Native moduleのbuildとSmoke成功

以上によりPhase 2を完了とする。Docker Engineがないため、Linux Native moduleを含む実Image buildは引き続き未実施である。
