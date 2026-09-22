# 外部GPTレビュー統合 2026-09-22

- 対象Snapshot: public GitHub Repositoryのcommit `b3ee38d`
- 確認日: 2026-09-22
- 目的: 4件の外部評価を事実確認し、設計計画へ反映する

外部レビューは設計要件ではなく、問題候補を発見するための資料として扱う。提案はV2の実コード、既存Decision、現在の運用意図と照合してから採否を決める。

## 元レビュー

1. [Review A: 総合8.8/10](https://chatgpt.com/share/6ab20c85-4ac0-83e8-9fdb-b9726f56ab52?ogimg=plain)
2. [Review B: 総合7.3/10](https://chatgpt.com/share/6ab20ca4-f49c-83ee-8b7d-c7cece4146f0?ogimg=plain)
3. [Review C: 総合5.8〜6.3/10](https://chatgpt.com/share/6ab20cc6-c75c-83e8-bc10-1db8d9a4404c?ogimg=plain)
4. [Review D: 総合8.5/10前後](https://chatgpt.com/share/6ab20cd9-d060-83ee-9e6e-cc55d70135a9?ogimg=plain)

評価点は辛口指定の強さで大きく変わるため、絶対値は採用判断に使わない。複数Reviewで繰り返された具体的な指摘を重視する。

## Reviewごとの要旨

### Review A

最も肯定的な評価である。

- TS Adapter、KBC Protocol、Rust Coreの境界を高く評価した。
- crateを3つに抑えた構成、bounded Queue、共有HTTP、Session上限、Storage、Notification、Security、文書構成を良い点とした。
- 単一Command WorkerによるI/O待ちのHead-of-Line Blockingを将来課題とした。
- Motionの数MiB級binaryを現在のN-API/JSON経路へ流さないよう提案した。
- `ut`へMotionを直接追加してさらに肥大化させず、責務単位で分けることを提案した。
- `CURRENT_SYSTEM.md`が旧V1調査書に見えにくい点を指摘した。
- 現時点では全面的な並列化や大規模module分割は不要と評価した。

### Review B

Architectureの方向を評価しつつ、運用上の弱点を強く指摘した。

- 単一WorkerでHTTP Command、Reaction、軽量Commandまで直列になる点を最大の性能問題とした。
- TypeScript Event Dispatcher満杯時のfatal shutdownをoverload耐性の弱点とした。
- `SendAttachment.data`が`serde_json::Value`を通るため、Motion binaryでMemory負荷が増える可能性を指摘した。
- Notification、Storage、Session、Command moduleの肥大化を警告した。
- GitHub Storageは現在の低頻度設定には適するが、高頻度Dataへ拡大すべきでないとした。
- 最小CI、Protocol生成物同期確認、README、軽量Metricsを提案した。
- Action Queueの直列化とStorage Mutex中のNetwork I/Oは、利用量増加時の限界点と評価した。

### Review C

辛口指定に沿い、未完成部分と開発運用を最も厳しく評価した。

- 単一Command Workerと単一Action consumerを「全体1車線」と評価した。
- Event Dispatcher overflow時のshutdownを重大な負荷時リスクとした。
- 状態機械、複雑Parser、Storage競合のテストが少ないと評価した。
- GitHub Actions不在と公開履歴が2 commitだけである点を問題とした。
- 大きなRust module、文書の重複、README不在、Docker root実行を改善候補とした。
- Protocol境界、Security、GitHub Storageの競合・配送不明対策は高く評価した。
- 固定Bot管理者IDを外部設定やRoleへ変更するよう提案した。

### Review D

肯定的な評価と、実装に直接結び付く問題提起のバランスが取れている。

- Rust正本のProtocol、TS型生成、薄いN-API、Capability注入、Session所有権を高く評価した。
- Event Dispatcher overflowと単一WorkerのHead-of-Line Blockingを主なRiskとした。
- ただし低トラフィック時の直列実行には、安全性と予測可能性の利点もあると評価した。
- command、queue、HTTP、Discord Actionの軽量計測を先に行うよう提案した。
- Docker buildがProtocol型を再生成しないため、生成物driftをCIで検査するよう提案した。
- `rust-version = 1.89`とDocker Rust 1.98の運用方針を揃えるよう提案した。
- Bot管理者IDを将来Policy化する案を示した。

## 4件で共通して評価された点

- TypeScriptをDiscord Adapterへ限定したこと
- Rust CoreをBot本体としたこと
- Rust定義をProtocolの正本にし、TS型を生成すること
- Command固有N-APIを増やしていないこと
- Queue、HTTP、Session、Response sizeを有限化したこと
- shared mutable stateとcrate分割を増やしすぎていないこと
- Webhook認証、Body上限、Secret保護、mention抑止
- GitHub StorageのSHA競合、write直列化、通知重複回避
- AGENTS、Architecture、Decision、Implementation資料による設計共有

これらはV2で維持する。外部レビューを理由にArchitecture全体を作り直さない。

## 指摘の実コード確認

| 指摘 | 確認結果 | 判断 |
|---|---|---|
| Command Workerが直列 | 事実。HTTP await中も次Eventを処理しない | Riskとして観測する。MotionはTask Runtimeへ分離する |
| Discord Actionが直列 | 事実。全Actionを1件ずつ実行する | 順序保証の利点がある。計測なしに並列化しない |
| Event Dispatcher満杯で停止 | 事実かつ既存Decisionどおり | Riskは認めるが、Reaction/ActionResultのsilent drop案は採用しない |
| Motion binaryがJSON的配列を通る | 事実。`CoreAction`を`serde_json::Value`へ変換後、TSが`Buffer.from()`する | Motionではこの経路を使わない |
| `.github` CIがない | 事実 | 最小CIを追加候補とする |
| Protocol生成物driftをDockerが検出しない | 事実 | CIで生成後差分確認を行う候補とする |
| Rust moduleが大きい | 事実。ただしSizeだけでは責務違反を意味しない | 対象変更時に自然な責務境界だけ分ける |
| Storage Mutex中にNetwork I/O | 事実。現在はStorage操作の直列性を保証する | 低頻度のため維持し、contention計測後に見直す |
| Testが`smoke.ts`だけ | 不正確。Rustに9件の代表Testがある | 状態機械の重大境界だけ追加を検討する |
| Code Repositoryがprivateという資料 | Code Repositoryはpublic。Storage用Data Repositoryはprivate | 両者を区別し、誤記だけ訂正する |
| Git履歴が2 commit | public Snapshotでは事実 | 公開済み履歴は書き換えず、今後を小さなcommitにする |
| READMEがない | 事実 | 公開Repositoryの入口として追加候補とする |
| LICENSEがない | 事実 | 公開とOpen Source許諾は別。利用許諾を決めるまで自動追加しない |

## 採用する改善

### Motion開始前または同時に行う

1. Motion出力を通常の`SendAttachment.data`へ載せない。
2. Rustが所有する一時Fileを新しいAttachment ActionでTSへ渡し、Discord送信完了後にcleanupする。
3. Task RuntimeでMotionを既存Command Workerから分離する。
4. Taskのqueue wait、処理段階、FFmpeg、upload、RSSを計測する。
5. Task終端と一時File cleanupに少数の恒久Testを置く。

### Motion本環境投入までに行う候補

1. `generate:protocol`後の差分、Rustfmt、Clippy、Rust Test、TS typecheckを確認する最小CI。
2. Event Dispatcherのoutstanding high-waterとoverflow、Command時間、Discord Action時間の軽量structured log。
3. Protocolの大容量File Actionに必要なversion更新と型生成同期確認。
4. Motion codeを`ut/mod.rs`・`tut/mod.rs`へ混在させず、独立Motion moduleへ置く。

### 計測後にだけ行う

- 通常I/O Commandの有限並行化
- Discord ActionのRequestまたはMessage単位の有限並行化
- Event Dispatcherの複数laneまたは予約容量
- Storage state lockとwrite serialization lockの分離
- Notification、Storage、Session、既存Commandの追加分割
- Docker build cache、non-root化、Rust version方針の変更
- GitHub Storageから別Storageへの移行

## 採用しない提案

### Bot管理者仕様の変更

固定Bot管理者、健康維持メンテナー、Roleの扱いには運用上の意図がある。外部Reviewの、管理者IDをConfig、Storage、Discord Role、汎用Authorization層へ変更する提案は採用しない。既存requirementsと利用者の明示指示を正とし、このReviewを理由に仕様変更しない。

### 負荷時の重要Event drop

`reactionAdd`や`actionResult`をdropするとDiscordとCoreの状態が分離する。Queue満杯時に新規Message、Reaction、Action結果を一律dropする案は採用しない。Event Dispatcherの変更には、Event class、順序、予約容量、busy応答、shutdown整合性を含む別Decisionが必要である。

### 計測前の全面並列化

現在の直列所有には、Session順序、低Memory、単純なshutdownという利点がある。全Commandや全Actionの並列化は採用しない。MotionをTask Runtimeへ分離し、通常Commandのqueue waitを計測してから有限並行性を判断する。

### 一括リファクタリング

File sizeだけを理由に、全Command、Notification、Storage、Sessionを同時分割しない。機能変更で明確な責務境界が現れたFileだけを分ける。

### 直ちにDatabaseへ移行

GitHub Storageは低頻度の設定と通知Checkpointに限定され、現時点の要件に合っている。高頻度Task履歴、Metrics、Cacheを保存し始めるまでは別Databaseへ移行しない。

### 公開Git履歴の書き換え

初回公開が大きなSnapshotだった事実は変えない。rebaseやforce pushで履歴を作り直さず、今後の変更を小さな目的単位にする。

## 設計への反映順

```text
Motion large-binary境界
→ Task Runtime
→ Motion renderer / encoder
→ TaskとQueueの計測
→ 最小CI・Protocol drift検査
→ 本環境観測
→ 通常Command / Action / Dispatcherの並行性判断
→ 必要な箇所だけ単純化・分割
```

Reviewの指摘を理由に、Motionと無関係な大規模改修を先に実施しない。
