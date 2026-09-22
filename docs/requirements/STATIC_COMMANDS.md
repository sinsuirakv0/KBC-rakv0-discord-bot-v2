# 静的Command仕様

## 対象

| Command | Response |
|---|---|
| `asset` | `content/responses/asset.txt` |
| `home` | `content/responses/home.txt` |
| `ping` | `content/responses/ping.txt` |
| `skb` | `content/responses/skb.txt` |
| `skdsite` | `content/responses/skdsite.txt` |

Response File名をCommand名とする。CommandはASCIIの大文字小文字を区別せず、すべてGuild限定とする。

## 入力と出力

- 通常実行では対応するResponse本文を同じChannelへ1件送信する。
- 第一引数が大文字小文字を問わず`help`なら、`content/help/<command>.txt`を送信する。
- helpが存在しない静的Commandでは通常Responseへfallbackする。
- `help`以外の引数は無視する。
- 起動後のCommand実行中にはContent Fileを再読込しない。

Local Discord Adapterでは、送信本文の先頭に`[local] `が付く。
