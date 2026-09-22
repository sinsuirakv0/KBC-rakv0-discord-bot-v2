# ping Command仕様

## 入力

- Prefixは`o.`。
- Command名`ping`はASCIIの大文字小文字を区別しない。
- 第一引数が`help`の場合は、大文字小文字を問わずpingのhelpを表示する。
- それ以外の引数は無視する。
- Guild内のMessageだけを処理し、DMでは応答しない。

例:

```text
o.ping
o.PING ignored
o.ping help
```

## 出力

Coreは同じChannelへの`sendMessage`として次を返す。

```text
pong!!
```

返信本文の正本は`content/responses/ping.txt`とし、起動後のCommand実行中にはFileを再読込しない。

Local Discord Adapterでは環境識別子が付くため、Discord上の表示は次になる。

```text
[local] pong!!
```

`o.ping help`の本文は`content/help/ping.txt`を正本とする。
