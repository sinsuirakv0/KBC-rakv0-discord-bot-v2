# Phase 8 作業チェックポイント

- 状態: 完了
- 最終更新: 2026-09-21
- 対象: `item` / `sale` / `gatya` と、その移植に必要な共通基盤

## 確定した方針

- 旧Repositoryは読み取りだけに使用し、変更しない。
- CommandごとのHTTP Clientは作らず、timeout・最大Response size・同時実行数を有限にする共通`HttpService`をRust Coreへ置く。
- 1 Commandから複数Messageを返せるようにするが、出力Action数には固定上限を設ける。
- `item`、`sale`、`gatya`のデータ取得・検証・整形はRust Coreへ置き、TypeScript Discord AdapterへCommand固有処理を追加しない。
- 3 Commandで重複する日時・Message分割等だけを共通化し、巨大なSchedule FrameworkやCache Frameworkは先に作らない。
- `sale <文字列>`は検索結果一覧まで移植する。9件以下のReaction選択と選択後の詳細表示は、複数Eventを跨ぐためPhase 9のSession Managerと同時に移植する。
- 旧実装にCacheがなく、更新頻度・TTL・stale方針も未確定なため、このPhaseではCacheを追加しない。

## 作業状況

- [x] 旧`item`の仕様・実装調査
- [x] 旧`sale`の仕様・実装調査
- [x] 旧`gatya`の仕様・実装調査
- [x] 共通HTTP基盤
- [x] 有限な複数Action出力
- [x] 共有Schedule補助処理
- [x] `item`移植
- [x] `sale`移植（Reaction選択を除く）
- [x] `gatya`移植
- [x] 最小検証
- [x] 実Discordで旧本番との比較
- [x] 設計・仕様・実装Document更新
- [x] BOM・秘密情報・旧Repository非変更の最終監査

## 自動検証

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p kbc-core`（5件成功）
- `npm run typecheck`
- `npm run smoke:runtime`
- `cargo build --workspace --release --locked`
- `docker build --progress=plain -t kbc-rakv0-discord-bot-v2:local .`
- Linux Container内の`Runtime smoke passed.`
- Linux ContainerでDiscord Adapter起動、15秒間の稼働、SIGTERMによる正常停止
- 公開dataを使った旧版との出力比較: 予定、詳細、JSON、Raw、名称検索、mode指定がすべて完全一致
- 日本語FileのBOM、秘密値の`.env`外非重複、旧Repository未変更を確認

Windows release buildとLinux Container buildの両方が成功した。BIOSでは`Intel(R) Virtualization Technology`と`Intel(R) VT-d`が有効で、Windows Update完了後にWSL 2とDocker Desktop 4.91.0が正常起動した。Docker Engine 29.8.0のLinux/amd64環境で、生成ImageのRuntime SmokeとDiscord Adapterの起動・停止を確認した。

Docker Desktop初回起動時は、Windows Update前に作られたUnix Socketがアクセス不能になっていた。Docker停止後に`%LOCALAPPDATA%\Docker\run`と`%LOCALAPPDATA%\docker-secrets-engine`を退避して空Directoryを再作成することで解消した。退避Directoryは元のDocker LocalData内に残している。

Local `.env`の`DISCORD_TOKEN`は引用符付きであり、`docker run --env-file .env`では引用符もTokenの一部として渡される。Container確認では値を表示せずに引用符を除去し、`-e DISCORD_TOKEN`で環境変数を継承させた。本番環境でもTokenはImageへ含めず、実行環境のSecretとして注入する。

## 中断時の再開地点

Phase 8は完了した。実Discordで`o.item`、`o.sale`、`o.gatya`の正常動作と`[local]`表示を確認済み。Docker image build、Container内Runtime Smoke、Discord AdapterのContainer起動・正常停止も確認済み。Local Botと検証Containerは停止済みである。

自動検証では予定、詳細、JSON、Raw、名称検索、mode指定を旧版とメッセージ単位で比較し、すべて一致した。`sale`のReaction選択は未実装である。

次の作業はPhase 9である。`sale`の9件以下の検索結果に対するReaction選択を最初の実利用者として、最小Session Managerの要件・旧実装・Discord Adapterの既存`reactionAdd`経路を調査する。

実Discord確認ではユーザーが3 Commandの正常動作を確認した。
