# Command開発規約

Commandを追加・変更する場合はこの文書を読むこと。

## 1. 基本

CommandはKBC Core上で動くFeatureである。

Command自身が独自Runtimeを作らない。

---

## 2. 新規Command追加時の確認

実装前に以下を判断する。

### 通常Commandで完結するか

短時間・軽量処理ならCommand内で完結させる。

### 共通Serviceが必要か

HTTP、Cache、Storage、Asset取得等は既存Serviceを利用する。

外部Text dataの取得には共有`HttpService`を使う。Command側はURLとdata検証を担当し、Client、同時実行制御、timeout、Response size上限を作り直さない。

### Task化が必要か

以下の場合はTask Runtimeを検討する。

- CPU負荷が高い
- 長時間動作する
- Queueが必要
- Timeoutが必要
- Cancelが必要
- Progress表示が必要
- External Processを使用する

### Sessionが必要か

複数Discord Eventを跨ぐ場合はSession Managerを利用する。

---

## 3. 禁止事項

原則としてCommand内で以下を作らない。

```text
discord.js依存
独自HTTP Client
独自Connection Pool
独自Cache Framework
独自Job Queue
独自Timeout System
独自Progress System
無制限spawn
巨大Global mutable state
```

---

## 4. 共通化

同じ処理が2～3箇所にあるだけで即Framework化しない。

共通責務として明確になった場合のみShared Service等へ昇格する。

---

## 5. ファイル分割

現行コードの、

```text
command
data-source
domain
parser
formatter
types
```

の分離は参考にする。

ただし小さいCommandへ不要なファイルを大量作成しない。

責務が明確に分かれた時だけ分割する。

---

## 6. Discord出力

Commandはdiscord.js Objectを生成しない。

KBC ProtocolのActionを通じてDiscord操作を依頼する。

複数Messageが必要な場合は`CommandOutput`へ順番に追加する。1 CommandのAction数は最大32件で、長文は共有のUTF-16単位の分割処理を使う。

---

## 7. Error

技術的Errorとユーザー向け表示を分離する。

内部詳細やURL、Stack Trace等をそのままDiscordへ返さない。

---

## 8. Command追加時のDocument

ユーザーから見た仕様がある場合:

```text
docs/requirements/
```

重要な設計判断を伴う場合:

```text
docs/decisions/
```

複雑な実装説明が必要な場合のみ:

```text
docs/implementation/
```

へ記録する。

---

## 9. Test

Command追加を理由に巨大な専用Test Suiteを作らない。

詳細は`TESTING.md`を参照する。
