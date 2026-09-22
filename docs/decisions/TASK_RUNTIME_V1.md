# Task Runtime V1設計判断

## 状況

Phase 10の最初の実利用者は、`ut`と`tut`のモーション生成である。旧版はNode.js Worker threadとFFmpegを使い、共有の直列Queueで処理している。

現時点のV2には`ut`/`tut`の検索、選択、Asset解決、Motion planがまだない。利用者なしにTask APIを実装すると、実際の入力・結果・進捗契約と合わないFrameworkを固定する危険がある。

## 決定

Task Runtimeの設計境界はPhase 10で確定するが、production codeは最初のMotion task実装と同じ作業単位で追加する。

初期Task Runtimeは次に限定する。

- Core内部の`TaskId`
- 実行中1件と有限な待機Queue
- Queue満杯時の即時busy応答
- Task全体timeout
- 進捗が一定時間ない場合のstall timeout
- timeout・shutdown・cleanupから使うCancellation token
- boundedな進捗通知と同一内容のcoalescing
- 完了・失敗・timeout・cancelの一度だけの終端処理
- 一時File、CPU worker、外部Processを確実に解放するcleanup契約

Discordからの手動Cancel UIと`TaskCancellation` Protocol Eventは旧仕様にないため、初期版へ追加しない。必要になるまでTask IDもProtocolへ出さない。

## Motionから得た初期値

- 同時実行: 1件
- 全体timeout: 10分
- stall timeout: 60秒
- 進捗編集の最短間隔: 2秒
- FFmpeg thread: 1
- Frame rate: 30 fps

旧版の待機Queueには容量上限がない。V2では保守的な固定上限を設定するが、具体的な待機件数はMotion planとAssetの保持量を確認してから確定する。Queueには大きなAsset byte列を入れず、識別子と検証済みplanだけを保持する。

## Actionとの接続

Motion Commandは最初に進捗Messageを`sendMessage`し、その`actionResult`で得たmessage IDへTaskを結び付ける。

```text
Command
→ 進捗Message送信
→ actionResult(messageId)
→ Task Queue登録
→ editMessageによる進捗
→ sendAttachment
→ editMessageによる完了または失敗
```

Taskの進捗・完了Actionは元Commandの`RequestId`を維持する。TypeScript AdapterはTask状態を持たず、既存のActionを実行するだけとする。

## Backpressure

Task Queueと進捗Event Queueはboundedとする。進捗は最新状態だけに価値があるため、遅いDiscord編集に合わせて全更新を蓄積しない。同一Taskの未送信進捗は新しい内容で置き換える。

完了、失敗、timeout、cancelは破棄してはならない。進捗経路とは分けるか、終端Event用の予約容量を持たせる。

## CancellationとCleanup

timeoutまたはshutdown時は最初にCancellationを通知し、CPU workerとFFmpegを停止する。その後、一時Fileを削除する。cleanup完了前にTask slotを次へ渡さない。

RustのFutureをdropするだけでは`spawn_blocking`や外部Processは止まらないため、Motion rendererはCancellationを明示的に確認し、外部Process killとworker終了待ちを実装する。

## Resource Managerとの境界

Task RuntimeはTaskの状態、Queue、timeout、進捗、終端を管理する。Phase 11のResource ManagerはCPU-heavy permitとExternal-process permitだけを管理する。Taskごとに独自Semaphoreを作らない。

## 見送るもの

- Priority Scheduler
- Task永続化と再開
- 無制限Queue
- Discord用Cancel button
- CommandごとのTask runner
- 汎用的な文字列Key Service locator
