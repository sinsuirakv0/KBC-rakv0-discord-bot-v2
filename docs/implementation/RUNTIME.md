# AppRuntime / ActionBus実装

## 実装範囲

Discord非依存の`kbc-core`へbounded Event Queue、Worker、ActionBus、shutdown lifecycleを実装し、Phase 5でCommandRuntimeをWorkerへ接続した。Command処理の詳細は`docs/implementation/COMMAND_RUNTIME.md`に記録する。

## Rust関数と相互関係

### AppRuntime

- `RuntimeConfig::default()`: Event Queue 64件、Action Queue 16件、Content root `content`を返す
- `AppRuntime::start(config)`: 容量を1〜1024件で検証し、Content Catalog、2つのQueue、停止通知、Workerを作る
- `AppRuntime::submit_event(event)`: Version検査後、Event Queueへ非同期送信する
- `AppRuntime::next_action()`: ActionBusから1件受け取る。停止通知を受けた場合は`None`を返す
- `AppRuntime::shutdown()`: 停止状態を一度だけ確定し、Worker終了を待つ
- `validate_capacity(queue, capacity)`: Queue容量の共通検証
- `wait_for_shutdown(receiver)`: Tokio watch channelによる停止待ち
- `run_worker(...)`: EventをCommandRuntimeへ渡し、返されたActionをAction Queueへ直列送信するWorker loop

### ActionBus

- `ActionBus::new(capacity)`: bounded channelを作り、受信側を`ActionBus`、送信側をWorkerへ返す
- `ActionBus::next_action()`: Mutexで受信を直列化し、次のActionを待つ

Action送信側を`ActionBus`自身へ保持しない。Workerが終了するとsenderがdropされるため、`next_action()`は予期しないQueue切断を`ActionQueueClosed`として検出できる。

## Bridgeとの関係

```text
TypeScript createCore()
→ N-API create_core()
→ AppRuntime::start()

submitEvent() → submit_event() → Event Queue
nextAction()  ← next_action()  ← ActionBus
shutdown()    → shutdown()     → Shutdown signal → Worker join
```

`NativeRuntimeConfig`はTypeScriptのoptional設定をRustの`RuntimeConfig`へ変換する。Queue容量の未指定項目にはRust側の既定値を使う。Discord Adapterはbuild成果物からRepositoryまたはContainerの`content`を絶対Pathで解決し、`contentDirectory`として渡す。Protocol入力、Runtime、Serde変換のerrorは`kbc-node`だけでN-API errorへ変換する。

## Lifecycle

停止はTokio watch channelでQueue待機中の処理へ通知する。各`select!`では停止を優先し、`submit_event()`とWorkerを速やかに終了させる。`next_action()`は停止を正常終了として`None`へ変換する。

Workerの`JoinHandle`はMutex内の`Option`として一度だけ取得する。最初の`shutdown()`がWorkerをjoinし、同時または後続の呼び出しは同じ完了を待った後に成功する。

## 最小Smoke

`apps/discord/src/protocol/smoke.ts`の1系統だけを維持する。

1. 容量1件のRuntimeを作る。
2. `o.HOME ignored`で静的Responseを確認する。
3. `o.PING HELP`で静的Command helpを確認する。
4. `o.help ignored`でhelp indexを確認する。
5. `nextAction()`待機中にshutdownする。
6. 待機結果が`null`になることを確認する。

Queueの個別分岐や単純なgetterのUnit Testは追加しない。

## 後続Phaseへ残すこと

- Phase 4: Discord Event / APIとProtocolの変換
- 実際のAttachment Action導入時: `Uint8Array` / `Buffer`変換とsize上限の確認
- graceful drainが必要になった場合: 上限時間と失敗時Policyを含む別設計

## Phase 3検証結果

2026-09-21時点で、次を確認した。

- 配置済みWindows release Native moduleによるRuntime Smoke成功
- `npm run check:rust`成功
- `npm run typecheck`成功
- `cargo fmt --all -- --check`成功
- 全targetのClippyをwarning禁止で実行し成功
- Windows release Native moduleのbuild成功

以上によりPhase 3を完了とする。
