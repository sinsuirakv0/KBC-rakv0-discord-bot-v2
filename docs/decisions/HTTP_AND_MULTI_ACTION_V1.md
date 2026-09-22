# 共有HTTPと複数Action V1

## 決定

- `item`、`sale`、`gatya`はRust Coreの共有`HttpService`を使う。
- ClientはRuntimeごとに1つ作り、既定で同時4Request、10秒timeout、Response 8 MiBに制限する。
- 1 Commandの出力は最大32 Actionとし、Action Queueへ順番に投入する。
- 長文はDiscord上限より余裕を持ったUTF-16 1800単位で分割する。
- 外部Event dataにはCacheを追加しない。`ut`/`tut` metadataだけは旧仕様に合わせた検証済みRemote snapshotを持つ。

## 理由

3 Commandは同じ公開Repositoryから複数Fileを取得する。CommandごとにClientやSemaphoreを持つと、Bot全体の同時実行数を制限できない。共有ServiceならHTTP接続を再利用しながら、待機数とResponse memoryを有限にできる。

Phase 8時点の代表的な最大Responseは`sale.json`の約306 KiBだった。Phase 10で追加した`character-index.json`は2026-09-21実測で625,735 byteあり、512 KiBを超える。さらに同じServiceで有限なAsset添付を取得するため、既定値を8 MiBへ変更し、設定可能上限16 MiBは維持する。複数返信の最大実測は`sale 17000`の12件だったため、32 Actionを初期上限とした。

`item`、`sale`、`gatya`はCommand実行ごとに取得し、Cacheしない。`ut`/`tut`は旧版で10分TTL、ETag/Last-Modified再検証、更新失敗時の検証済みstale利用が定義済みだったため、その2系統だけへ適用する。Asset存在結果は10分TTL、最大512件とし、file本体はCacheしない。

## PlatformごとのTLS

Windows開発環境は`native-tls`、Linux Docker buildは`rustls-tls`を使う。WindowsのGNU LLVM toolchainで`ring`のC buildを要求せず、Linux本番ではOpenSSL runtime依存を増やさないためである。HTTP API自体は`HttpService`へ閉じ込め、Commandから差を見せない。

## Lifecycle

Command実行はWorkerのshutdown待機と競合させる。停止通知が来た場合はCommand Futureをdropし、HTTP RequestとSemaphore待機を中断する。生成済みActionもdrainせず、既存の高速停止方針を維持する。

## 見送ったもの

- 汎用Cache Framework
- CommandごとのHTTP Client・timeout
- 無制限Action list
- `sale`検索のReaction Sessionを模倣する一時実装

`sale`のReaction選択は複数Discord Eventを跨ぐため、Phase 9のSession Managerで扱う。
