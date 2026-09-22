# Phase 6 作業チェックポイント

- 状態: 完了
- 最終更新: 2026-09-21
- 対象: Configuration / Content Catalog

## 調査結果

- 主計画はPhase 6の候補としてHTTP、Configuration、Metricsを挙げるが、実際に必要なものだけを導入する方針である。
- Phase 0の推奨移行順では、CommandRuntimeと`ping`の次は`content/responses`・`content/help`のFile loaderである。
- 現行の静的返信は起動時に読まれ、BOM・改行を正規化し、不正File名と空Responseを拒否する。
- 次の単純なCommand群はLocal contentを必要とし、HTTP・Clock・Metricsはまだ利用者がいない。

## 確定した方針

- Phase 6ではcontent path設定と、起動時に不変Snapshotを作るContent Catalogだけを実装する。
- `AppRuntime::start()`でContentを読み、Commandへ必要な文字列だけを注入する。
- 既存`ping`の返信をhardcodeから`content/responses/ping.txt`へ切り替え、Runtime Smokeを維持する。
- 読み込むFile数と1 FileのByte数に上限を設ける。
- HTTP、Clock、Metricsは利用するCommandの直前まで実装しない。

## 作業状況

- [x] 関連設計と旧Content loaderの読み取り調査
- [x] Phase 6の範囲確定
- [x] Content Catalog実装
- [x] Runtime ConfigurationとN-API接続
- [x] 既存ContentのV2配置
- [x] `ping`へのContent注入
- [x] Compiler・Clippy・format・Smoke検証
- [x] Architecture・実装Document更新
- [x] BOM・旧Repository非変更の最終監査

## 完了確認

- `content/responses` 5件と`content/help` 15件をV2へ配置した。
- ContentはDirectoryごとに最大128件、1 File最大64 KiBとして起動時に検証する。
- `contentDirectory`をTypeScriptからN-API、Rust `RuntimeConfig`まで接続した。
- `ping`の返信をContent Fileから注入し、DebugとReleaseのRuntime Smokeで`pong!!`を確認した。
- Compiler、TypeScript typecheck、rustfmt、Clippyを通過した。
- HTTP、Clock、Metricsは未使用のため追加しなかった。
- 旧Repositoryは読み取りだけで変更していない。

次回はPhase 7の対象Commandを確定する。現在の1 Action制約内で進めるなら`help`と静的Command群が最小であり、`item`へ進む場合はboundedな複数Action出力とHTTP Serviceを先に設計する。
