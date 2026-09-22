# テスト方針

## 1. 最重要原則

**テストは超最小限とする。**

KBC Botでは、テスト数・Coverage率・Test Code量そのものを品質指標としない。

Rustの型システム、Ownership、enum、Result、Compilerによって保証できることを大量のUnit Testで二重確認しない。

---

## 2. テストを追加する条件

恒久テストは原則として次の場合だけ追加する。

### Protocol境界

TypeScript ↔ Rust間の契約破壊が重大なため、最小限のSmoke Testを維持する。

### Runtime基本経路

最低限、

```text
Event
→ Command
→ Action
```

が成立すること。

### 複雑なPure Logic

アルゴリズムが複雑で、コンパイル成功だけでは正しさを保証できないもの。

### 重大なRegression

過去に実際に発生した重要Bugで、再発可能性・被害が大きいもの。

---

## 3. 原則テストしないもの

以下へ「念のため」テストを追加しない。

```text
getter
setter
単純な型変換
明白なenum分岐
Compilerが保証する状態
単純なFormatter
単純なWrapper
```

---

## 4. Commandごとの巨大Test Suite

Commandを追加するたびに、

```text
xxx.test
```

を大量に増やす方式を採用しない。

Commandは原則として、

```text
compile
必要最小限のsmoke
必要なら代表入力で確認
```

で検証する。

---

## 5. Migration Test

移行期間のみ必要なCompatibility Testは作成してよい。

ただし恒久テストと区別する。

移行完了後に価値の低いCompatibility Testを削除してよい。

---

## 6. Compilerを第一防衛線とする

日常的な検証では、

```text
cargo check
cargo clippy
cargo fmt --check
TypeScript typecheck
```

を重視する。

毎回すべてを実行する必要はない。

変更範囲に応じて最小限を選択する。

---

## 7. Test追加時の質問

新しいテストを書く前に必ず考える。

1. 型で防げないか。
2. Compilerで防げないか。
3. 実際に壊れやすい境界か。
4. 将来このTestを保守する価値があるか。
5. 単純なSmokeで十分ではないか。

答えが曖昧ならTestを追加しない。

---

## 8. 目的

Test Codeそのものを減らすことが目的ではない。

> 少数の価値の高いテストだけを長期維持する。

ことを目的とする。
