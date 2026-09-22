# 性能・Resource設計規約

## 1. 基本思想

性能改善は推測ではなく計測に基づく。

```text
Measure
→ Bottleneck特定
→ Optimize
→ Measure Again
```

Rustで書いたこと自体を高速化の根拠にしない。

---

## 2. 想定環境

KBC Botは限られたCPU・RAM環境で運用される可能性を前提とする。

「サーバー性能を上げれば解決する」は基本設計としない。

---

## 3. AsyncとCPU処理

I/O:

```text
HTTP
Storage
Network
```

はasync runtimeを利用する。

CPU負荷の高い処理を無制限にasync executorへ投入しない。

---

## 4. 並列性

並列数を増やせば高速になるとは限らない。

低CPU環境では、2件のHeavy Taskを同時実行すると両方が遅くなる場合がある。

Resource Managerで同時実行数を管理する。

---

## 5. Memory

Hot Pathでは、

- 不要なclone
- 大きな一時Vec
- 全件Buffering
- Binaryの不要コピー

を避ける。

ただし小さなAllocationを避けるためにコードを極端に複雑にしない。

---

## 6. Streaming

大きなデータについては、

```text
produce
→ process
→ consume
```

のstreamingを優先する。

すべて生成してから次段へ渡す設計を無条件に採用しない。

---

## 7. Backpressure

Consumerが遅い場合、Producer側も待つ構造を優先する。

Memoryへ無制限に蓄積しない。

---

## 8. Cache

Cacheは必ず有限とする。

検討事項:

```text
TTL
最大件数
最大Memory
Invalidation
Stale Policy
```

---

## 9. N-API

N-API境界をHot Loop内で繰り返し跨がない。

可能な限り大きな処理単位でRustに任せる。

---

## 10. Benchmark

Performance関連変更は原則release buildで比較する。

必要な場合だけ一時Benchmarkを作る。

すべてのFeatureへ恒久Benchmarkを作らない。

---

## 11. Metrics

共通Runtimeで最低限、

```text
command total
queue wait
HTTP
processing
storage
external process
```

を観測できる設計を目標とする。

詳細な内部Timingは実際に問題が発生したFeatureへ追加する。
