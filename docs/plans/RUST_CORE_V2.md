# KBC Rust Core V2 実装計画

## 1. この計画の目的

現行コードを単純にTypeScriptからRustへ翻訳しない。

現行コードを参照実装・仕様資料として利用しながら、新しい長期Runtimeへ機能を載せ直す。

実装開始前にCodex自身も実コードを調査し、この計画との差異やより良い案を提示する。

---

# 2. Codexとの共同設計

各大Phase開始時にCodexは、

1. 対象コードを読む。
2. 関連requirements / decisionsを読む。
3. 現計画との矛盾を確認する。
4. より単純・高速・堅牢な案があれば提示する。
5. 実装方針を確定する。
6. 実装する。
7. 最小限の検証を行う。

計画書を絶対視して、実コードに合わない構造を無理に導入しない。

ただし重要設計を勝手に変更しない。

---

# 3. Phase 0 — 現行環境調査

コード変更を最小限にする。

調査:

- Command一覧
- Discord依存
- Interactive処理
- HTTP
- Data Source
- Cache
- Storage
- Notification
- Background処理
- CPU-heavy処理
- Native Rust
- Docker
- Northflank前提
- Existing docs
- Dependency graph

成果物:

```text
現行Architecture Map
依存関係Map
設計上の問題一覧
V2計画との差異
Codexによる改善提案
```

Phase 0の目的は「実装」ではなく「最終設計確定」。

---

# 4. Phase 1 — Workspace Skeleton

新基盤の最小構造を作成する。

候補:

```text
apps/
└─ discord/

crates/
├─ kbc-protocol/
└─ kbc-core/
```

crateを増やしすぎない。

完了条件:

```text
Rust workspace build
TS build
基本Docker構成成立
```

---

# 5. Phase 2 — KBC Protocol

実装:

- CoreEvent
- CoreAction
- Protocol IDs
- Protocol Version
- N-API Bridge
- TS側Protocol Type

可能ならRust定義からTS型生成を行う。

最小検証:

```text
TS → Rust → TS
```

1系統のSmokeのみ。

---

# 6. Phase 3 — AppRuntime / ActionBus

実装:

```text
AppRuntime
submitEvent
ActionBus
nextAction
shutdown
```

Dummy Eventを送信するとDummy Actionを返せる状態まで作る。

---

# 7. Phase 4 — Discord Adapter

現行discord.js処理をAdapterへ整理。

```text
Discord Event
→ CoreEvent

CoreAction
→ Discord API
```

Command固有処理を置かない。

実Discordで単純な疎通確認をする。

---

# 8. Phase 5 — CommandRuntime

実装:

```text
Command Parser
Registry
Dispatcher
CommandContext
Metadata
```

最初は最も単純なCommandを1つだけ載せる。

この段階でCommand RuntimeのAPIを確定する。

---

# 9. Phase 6 — 基本Service

実際に必要なものだけ導入する。

最初から巨大Service Frameworkを作らない。

候補:

```text
HttpService
Configuration
Metrics基礎
```

---

# 10. Phase 7 — 最初の実用Command

比較的単純な既存Commandを1つ移す。

このPhaseの目的は機能移植より、

> V2のCommand開発方法が実用的か確認する

こと。

必要ならCommand APIをこの段階で調整する。

---

# 11. Phase 8 — Asset / Cache

複数Commandで必要性が確認できた段階で実装する。

事前に巨大Cache Systemを作らない。

2026-09-21時点では、`item`、`sale`、`gatya`の移植に必要な共有HTTPと有限な複数Action出力を先に実装した。旧実装にCacheがなく、有効なTTL・最大件数・stale方針を決める根拠もないため、Cacheは必要性が確認できるまで延期する。

---

# 12. Phase 9 — Session Manager

Interactive Commandを移す直前に導入。

初期機能:

```text
SessionId
Owner
Expiry
Continuation
Cleanup
```

高度なSession Frameworkは不要。

---

# 13. Phase 10 — TaskRuntime

長時間・CPU-heavy等を必要とするCommandの直前に実装。

初期機能:

```text
TaskId
Bounded Queue
Timeout
Cancellation
Progress
```

---

# 14. Phase 11 — ResourceManager

最初は必要最小限。

```text
CPU-heavy permit
External-process permit
Queue capacity
```

Priority Schedulerや高度なMemory Budgetは必要性が判明するまで導入しない。

---

# 15. Phase 12 — 複雑Command

既存の複雑なCommandを順次V2へ載せる。

各Commandで新しい基盤を勝手に増やさない。

不足している共通機能が明確になった場合のみRuntimeを拡張する。

---

# 16. Phase 13 — Storage

既存Storage仕様をServiceとして統合。

現行の重要挙動を理解してから移す。

既存Storage方式自体の変更は別のArchitecture Decisionとして扱う。

2026-09-22時点で、private GitHub Repositoryを正本とするStorage、`push`、外部更新通知をRust Coreへ統合した。TypeScriptは認証付きHTTP境界とDiscord Action実行だけを担当する。詳細は`docs/implementation/STORAGE_AND_NOTIFICATIONS.md`と`docs/decisions/NOTIFICATION_RUNTIME_V1.md`を参照する。

---

# 17. Phase 14 — Heavy Processing

現在および将来の高負荷処理をTask Runtimeへ統合する。

特定Command専用Runtimeにはしない。

`utbench`は旧版で計測済みのためV2へ移植しない。MotionとTask RuntimeはV2の本環境稼働後に着手する。

確認対象:

- CPU負荷
- Queue
- Cancel
- Timeout
- Progress
- Cleanup
- External Process

---

# 18. Phase 15 — Observability / Performance

主要機能がV2上で動いてから本格最適化する。

計測:

```text
idle RAM
command latency
queue wait
CPU usage
heavy task latency
network wait
peak memory
```

測定結果に基づいて最適化する。

---

# 19. Phase 16 — Simplification

最後に、

- 一時Migration Code
- 不要Compatibility Test
- 古いInfrastructure
- 不要Dependency
- 不要なBridge

を整理する。

移行初期に既存コードを先に削除しない。

---

# 20. Phaseごとの作業単位

各Phaseは可能な限り小さなCommit単位にする。

```text
目的
↓
実装
↓
最小検証
↓
差分確認
↓
Document更新
```

巨大な一括変更を避ける。

---

# 21. テスト

本計画全体を通して`docs/engineering/TESTING.md`を適用する。

「大規模移行だから大量のテストを作る」という発想を採用しない。

型・Compiler・明確な境界を主な安全装置とする。

---

# 22. 計画変更

Codexがより適切な案を発見した場合、

```text
現在案
問題
代替案
利点
欠点
影響範囲
```

を示す。

重要なArchitecture変更は`docs/decisions/`へ残してから実装する。

---

# 23. 最終状態

```text
Discord
   ↓
discord.js
   ↓
TypeScript Discord Adapter
   ↓
KBC Protocol
   ↓
Rust KBC Core
   ├─ CommandRuntime
   ├─ SessionManager
   ├─ TaskRuntime
   ├─ ResourceManager
   └─ Shared Services
```

新Command追加時に必要なのは原則として、

```text
Command実装
↓
既存Serviceを利用
↓
必要ならSession / Taskを利用
↓
Registryへ登録
```

だけとする。

新しいCommandのたびに基盤を作り直さない。
