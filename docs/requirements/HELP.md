# help Command仕様

## 入力

- Prefixは`o.`。
- Command名`help`はASCIIの大文字小文字を区別しない。
- 引数は無視する。
- Guild内のMessageだけを処理し、DMでは応答しない。

## 出力

`content/help/index.txt`の本文を同じChannelへ1件送信する。起動後のCommand実行中にはFileを再読込しない。

Phase 7は段階移行中のため、indexにはV2でまだ実行できないCommandも含まれる。Contentは現行Botとの互換資料として維持し、本番切替前に実装済みCommandとの整合を改めて確認する。

Local Discord Adapterでは、送信本文の先頭に`[local] `が付く。
