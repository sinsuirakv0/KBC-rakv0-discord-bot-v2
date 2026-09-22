# Phase 3 作業チェックポイント

- 状態: 完了
- 最終更新: 2026-09-21
- 対象: AppRuntime / ActionBus

## 確定した方針

- `kbc-core`がAppRuntime、Event Queue、ActionBus、Lifecycleを所有する。
- Event QueueとAction Queueはbounded channelとする。
- 初期容量はEvent 64件、Action 16件、設定上限は各1024件とする。
- Queue満杯時は失敗させず、`submitEvent()`またはAction producerを待たせてbackpressureを掛ける。
- `shutdown()`は冪等とし、待機中の`submitEvent()`と`nextAction()`を終了可能にする。
- Phase 3のDummy経路は本文が`o.runtime-smoke`の`messageCreate`だけを同じchannel/contentの`sendMessage`へ変換する。
- Phase 2の`roundTripEvent()`は削除し、Runtime APIへ置き換える。

## 作業状況

- [x] 関連資料とPhase 2実装の再確認
- [x] Queue・Lifecycle設計の確定
- [x] Rust `ActionBus`実装
- [x] Rust `AppRuntime`実装
- [x] N-API `createCore` / `submitEvent` / `nextAction` / `shutdown`実装
- [x] TypeScript Bridge更新
- [x] Event → Action Smoke更新
- [x] Debug Smoke検証
- [x] Rust format・Clippy検証
- [x] Release build
- [x] Release native module配置
- [x] Release Smokeと最終一括検証
- [x] Architecture・実装Document更新

## 完了状況

Phase 3のコード、最小Smoke、設計資料、実装資料を確定した。DebugとReleaseのRuntime経路、Compiler、format、Clippyを確認済みである。

確認した項目は次のとおり。

- Debug Runtime Smoke
- Release Native module buildとRuntime Smoke
- `npm run check:rust`
- `npm run typecheck`
- `cargo fmt --all -- --check`
- 全targetのClippy（warningをerror扱い）
- 日本語を含むsource・documentのBOM
- 旧リポジトリの作業ツリーがcleanであること

## 実装済みファイル

- `crates/kbc-core/src/action_bus.rs`: bounded Action Queueの受信側。
- `crates/kbc-core/src/runtime.rs`: AppRuntime、Event Queue、worker、shutdown lifecycle。
- `crates/kbc-node/src/lib.rs`: `createCore`とRuntime非同期APIのN-API境界。
- `apps/discord/src/protocol/native.ts`: TypeScript側Runtime Bridge。
- `apps/discord/src/protocol/smoke.ts`: Event → Actionとshutdown待機解除の最小Smoke。

## 次の開始位置

次はPhase 4 — Discord Adapterの設計から開始する。開始前に現行リポジトリのDiscord Event受付、Discord API実行、起動・停止処理を読み取り調査し、TypeScriptへCommand固有Logicを持ち込まないAdapter境界を確定する。
