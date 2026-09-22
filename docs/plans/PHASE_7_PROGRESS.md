# Phase 7 作業チェックポイント

- 状態: 完了
- 最終更新: 2026-09-21
- 対象: help / 静的Command群

## 調査結果

- 現行の静的Commandは`content/responses/*.txt`をCommand名と返信として起動時に登録する。
- 対象は`asset`、`home`、`ping`、`skb`、`skdsite`の5件で、すべてGuild限定である。
- 第一引数が大文字小文字を問わず`help`なら、対応する`content/help/<command>.txt`を返す。
- 静的Commandのhelpが欠けた場合は、通常Responseへfallbackする。
- `o.help`は引数を無視し、`content/help/index.txt`を返す。
- 旧help indexには未移植Commandも記載されている。

## 確定した方針

- 現行Contentを変更せず、静的5 Commandと`help`をRust Coreへ登録する。
- Local ContentからCommand名が決まるため、Metadataのnameとaliasesを所有型へ変更する。
- 静的Command共通実装は1つだけ作り、Commandごとの重複Codeを作らない。
- Contentは起動時Snapshotのままとし、現行の実行時live reloadは採用しない。
- Phase 7時点ではhelp indexに未移植Commandが含まれることを明記し、本番切替条件とは分離する。
- 1 Command実行1 Actionを維持し、Queue・HTTP・Sessionは追加しない。

## 作業状況

- [x] 現行実装・要件・設計の読み取り調査
- [x] Phase 7実装方針確定
- [x] 所有型Command Metadataへの変更
- [x] 静的Command共通実装
- [x] help Command実装
- [x] RegistryとContent Catalog接続
- [x] Runtime Smoke更新
- [x] Compiler・Clippy・format・Release検証
- [x] Local用npm Script実装
- [x] `DISCORD_TOKEN`だけをV2 `.env`へコピー
- [x] `npm run local`起動・停止確認
- [x] 実Discord確認
- [x] Architecture・仕様・実装Document更新
- [x] BOM・秘密情報除外・旧Repository非変更の最終監査

## 完了確認

- 静的5 Commandと`help`をContentから登録した。
- Debug／Release Runtime Smokeで静的通常返信、個別help、help indexを確認した。
- Compiler、Clippy、rustfmt、TypeScript typecheckが成功した。
- `npm run local`だけでbuild、V2 `.env`読込、Local起動まで成功した。
- 実Discordで`o.ping`、`o.home`、`o.ping help`、`o.help`が`[local] `付きで応答した。
- `Ctrl+C`に対し`Received SIGINT; shutting down.`と`Discord adapter stopped.`を確認した。
- V2 `.env`は`DISCORD_TOKEN` 1項目だけを持ち、`.gitignore`対象である。
- Bot Processが残っていないこと、日本語FileのBOM、旧Repositoryの未変更を確認した。

次回はPhase 8の対象を再確認する。外部Data Commandへ進む場合は、実際の要件から最小HttpServiceと複数Action出力の必要性を判断し、利用者のないCache Frameworkは作らない。
