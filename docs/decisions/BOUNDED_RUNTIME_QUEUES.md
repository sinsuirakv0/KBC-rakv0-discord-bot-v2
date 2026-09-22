# Runtime Queueを有限にする

- 状態: 採用
- 決定日: 2026-09-20

## 決定

AppRuntimeのEvent QueueとAction QueueにはTokioのbounded channelを使う。

- Event Queue既定容量: 64件
- Action Queue既定容量: 16件
- 設定可能範囲: それぞれ1〜1024件
- 満杯時: dropや即時errorではなく、空きができるまでproducerを待たせる
- 停止時: 待機を解除し、未処理項目をdrainせず終了する

容量は`RuntimeConfig`で起動時にだけ設定する。自動拡張や無制限Queueは導入しない。

## 理由

Discord入力や将来のTask出力がconsumerの処理能力を超えても、Memory使用量を明確な上限内に保つ必要がある。待機によるbackpressureなら、一時的な遅延をproducerへ返しつつEventやActionを暗黙にdropしない。

EventとActionでは滞留特性が異なるため、容量を別々に設定できるようにする。既定値は初期運用値であり、性能保証値ではない。

## トレードオフ

- consumerが遅い間、`submitEvent()`やWorkerの完了も遅くなる。
- 小さすぎる容量はburst吸収力を下げ、大きすぎる容量はMemoryと停止時の未処理量を増やす。
- 停止時にdrainしないため、Queue内の処理は失われる。ただしPhase 3には永続化対象がなく、停止時間を有限に保つ方を優先する。

## 採用しなかった案

- unbounded channel: 負荷上限がなく、長期運用時のMemory増加を防げない。
- 満杯時の即時errorまたはdrop: 呼び出し側へ再送Policyが必要になり、初期Runtimeを複雑にする。
- 動的な容量変更: 計測値と運用要件がない段階では複雑さに見合わない。

## 変更条件

実運用のqueue wait、処理遅延、Memory使用量を計測し、既定容量が不適切だと確認できた場合に調整する。graceful drainや優先度制御が必要になった場合は、最大待機時間と公平性を含めて別のDecisionを作る。
