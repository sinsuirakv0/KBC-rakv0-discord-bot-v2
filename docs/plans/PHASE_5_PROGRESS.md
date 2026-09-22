# Phase 5 作業チェックポイント

- 状態: 完了
- 最終更新: 2026-09-21
- 対象: CommandRuntime / ping

## 調査結果

- 現行Parserは`o.`で始まる本文だけを処理し、Command名をlowercase化して引数を空白分割する。
- RegistryはCommand名とaliasをcase-insensitiveに解決し、重複登録を拒否する。
- 全CommandはGuild限定で、未知CommandとDMでは応答しない。
- `ping`は引数を無視し、`pong!!`を返す静的Commandである。

## 確定した方針

- `AppRuntime`が`CommandRuntime`を所有する。
- CommandRuntimeがParse、Resolve、Guild判定、Dispatch、Action envelope生成を担当する。
- Commandはasync対応の共通traitを実装する。
- Metadataはname、aliases、guildOnlyだけに限定する。
- CommandContextはPhase 5で必要なChannel IDだけを持つ。
- 1回のCommand実行は最大1 Actionとし、無制限Action Bufferを作らない。
- 最小Smokeを`o.PING ignored`から`pong!!`への経路へ更新する。

## 作業状況

- [x] 関連設計と旧Command基盤の読み取り調査
- [x] Command Runtime V1設計確定
- [x] Architecture Decisionとping仕様作成
- [x] Command API・Parser・Registry実装
- [x] CommandRuntime・AppRuntime接続
- [x] ping Command実装
- [x] Runtime Smoke更新
- [x] Compiler・Clippy・format・Release検証
- [x] Architecture・実装Document更新
- [x] 実Discord login確認
- [x] `o.PING ignored`の送受信確認
- [x] Signal shutdown確認

## 完了確認

- 旧環境のTokenはV2へ保存せず、検証Processの環境変数としてだけ渡した。
- 実Discordで`o.PING ignored`を送信し、`[local] pong!!`が1件返ることを確認した。
- 応答後の実行ログにerrorがないことを確認した。
- `SIGINT`に対して`Received SIGINT; shutting down.`、`Discord adapter stopped.`が出力されることを確認した。
- Phase 5完了時点でBot Processは停止済み。

次回はPhase 6 Basic Serviceへ進み、実際のCommand移植に必要な共通Serviceだけを追加する。
