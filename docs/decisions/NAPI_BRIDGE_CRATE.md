# N-API Bridge crateの分離

- 状態: 採用
- 決定日: 2026-09-20

## 決定

Rust workspaceを次の3 crateで開始する。

```text
kbc-protocol
kbc-core
kbc-node
```

- `kbc-protocol`: TypeScript AdapterとRust Core間のversion付きDTO
- `kbc-core`: DiscordとNode.jsに依存しないRuntimeとDomain Logic
- `kbc-node`: N-API変換とNode.js向けLifecycleだけを扱う薄いBridge

依存方向は次に限定する。

```text
kbc-node → kbc-core → kbc-protocol
         └──────────→ kbc-protocol
```

`kbc-protocol`と`kbc-core`から`napi`へ依存しない。

## 理由

2 crate構成ではN-API依存の置き場所が曖昧になり、ProtocolかCoreがNode.js固有型へ結合する。Bridgeを分けることで、Coreの単体利用、Protocolのversion管理、N-API変換を独立して変更できる。

## トレードオフ

- crateとbuild設定が1つ増える。
- DTOからN-API公開型への変換が必要になる。
- Node.js以外のAdapterを将来追加してもCoreを変更しなくてよい。

## 変更条件

Bridgeが単なる変換層ではなくDomain Logicを持ち始めた場合は責務を見直す。crate数を減らすためだけにCoreへN-API依存を移さない。
