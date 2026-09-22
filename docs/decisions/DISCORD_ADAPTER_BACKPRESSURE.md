# Discord Gateway入力を有限にする

- 状態: 採用
- 決定日: 2026-09-21

## 決定

TypeScript Adapterに容量64件の共有FIFOを1つ置き、Discord Gateway EventとAction Resultを登録順に`NativeCore.submitEvent()`へ渡す。drainは1つだけ実行する。

FIFOが満杯になった場合は新しいEventを黙ってdropせず、fatal errorとしてClientとCoreを停止する。Process managerによる検知と再起動を可能にする。

## 理由

Discord.jsのEventEmitterはasync listenerの完了を待たない。各listenerから`submitEvent()`を無制限に開始すると、RustのEvent QueueがboundedでもJavaScript側に未完了PromiseとDiscord Objectが無制限に滞留する。

共有FIFOにより、Memory上限とEvent順序を明示できる。Action Resultも同じ経路へ通すため、Gateway Eventとの投入順序を1箇所で管理できる。

## トレードオフ

- Discord Gateway自体を停止して再送させる仕組みではないため、overflowを起こしたEventは回復できない。
- fatal停止は可用性を一時的に下げるが、重要なReaction等をsilent dropしてCore状態とDiscord状態を不一致にするより検知しやすい。
- 直列drainは最大throughputを制限するが、初期段階では順序と有限性を優先する。

## 採用しなかった案

- async listenerから直接`submitEvent()`を呼ぶ: 未完了Promise数に上限がない。
- unbounded JavaScript Queue: Rust Queueをboundedにした意味が失われる。
- 満杯時のsilent drop: SessionやAction Resultを失ったことを検知できない。
- 初期段階から複数drain: Event順序が変わり、実測なしに複雑性を増やす。

## 変更条件

実運用でoverflowまたは継続的なqueue waitが観測された場合、Core処理、容量、複数drainの順で検討する。容量を増やすだけで問題を隠さない。
