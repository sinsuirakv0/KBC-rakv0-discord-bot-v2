# KBC Discord Bot 設計原則

## 1. 目的

KBC Discord Botは、現在存在するCommandだけでなく、今後数年間に追加される未知の機能まで同じ基盤上で運用できることを目標とする。

新基盤は単なる機能集合ではなく、軽量CommandからCPU負荷の高い長時間処理まで、安全かつ予測可能に実行するためのBot Runtimeとして設計する。

重視する優先順位は原則として以下とする。

```text
正しさ
↓
信頼性
↓
保守性
↓
計測された性能
↓
実装上の手軽さ
```

性能は重要だが、壊れやすい高速化は成功とは扱わない。

---

## 2. TypeScriptとRustの責務

### TypeScript

TypeScriptはDiscord Adapterである。

主に以下を担当する。

- `discord.js`
- Gateway
- Discordイベント
- Message / Reaction / Attachment
- Discord API操作
- Discord固有のエラー処理
- Discord EventからKBC Protocolへの変換
- Core ActionからDiscord API操作への変換

TypeScriptへCommandのビジネスロジックを蓄積しない。

### Rust

Rust CoreをBot本体とする。

以下を担当する。

- Command Runtime
- Parser
- Domain Logic
- Search
- Formatter
- HTTP
- Cache
- Storage
- Session
- Task Runtime
- Resource管理
- 高負荷処理
- Metrics
- Error Policy

---

## 3. Rustを使う目的

Rustを単なる高速化手段として扱わない。

Rustの、

- Ownership
- Borrowing
- enum
- Result / Option
- RAII
- 型安全性
- 明示的な並行性
- Memory Safety

を、長期運用の信頼性へ利用する。

可能な限り、

> 存在してはいけない状態を、型として表現不可能にする。

大量のbooleanやnullable fieldの組み合わせによって状態を表現しない。

---

## 4. 所有権

すべての重要な状態について「誰が所有するのか」を明確にする。

共有mutable stateを安全にすることより、共有mutable state自体を減らすことを優先する。

`Arc<Mutex<_>>` を便利なGlobal Stateとして乱用しない。

---

## 5. CoreはPlatform

新しいCommandを追加するたびに、

- Queue
- Cache
- HTTP Client
- Progress
- Timeout
- Session機構

を作り直さない。

Commandは既存RuntimeとServiceを利用する。

理想は、Command数が現在の数倍になってもCoreの基本構造がほぼ変わらないことである。

---

## 6. 有限なリソース

CPU、RAM、Network、Process、Queue、Session、Cacheはすべて有限である。

以下を原則禁止する。

```text
無制限spawn
無制限thread
無制限Queue
無制限HTTP並列
無制限Cache
無制限Session
無制限Buffer
```

低性能環境でも安定して動くことをアーキテクチャ要件とする。

---

## 7. 共通基盤を再利用する

複数Featureで必要なものはServiceとして共通化する。

ただし、将来使うかもしれないという理由だけで抽象化しない。

```text
simple now
replaceable later
```

を優先する。

---

## 8. 最適化

通常コードは、

- 読みやすい
- 安全
- 保守しやすい

ことを優先する。

計測によってHot Pathと確認された部分のみ、Allocation削減・Buffer再利用・SIMD・特殊なData Structureなどを検討する。

Rust Core全体を難解な低レベルコードにしない。

---

## 9. 長期目標

新機能が増えるほど基盤が複雑になるのではなく、基盤を再利用するほど新しい機能を追加しやすくなる状態を目標とする。
