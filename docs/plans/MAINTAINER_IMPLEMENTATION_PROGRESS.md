# Botメンテナー実装チェックポイント

- 状態: 実装・Local実Discord確認・本環境反映完了
- 最終更新: 2026-09-24

## 実装済み

- 固定Bot管理者3人の仕様を維持した。
- 全サーバー共通のユーザーID・ロールIDを`config/maintainers.json`へ保存する。
- `o.maint maintainer <ID>`、末尾`del`、`list`を追加した。
- `o.push`は固定Bot管理者に加え、登録ユーザー本人または登録ロール所持者が実行できる。
- `list`は実行Guildの実メンバーを解決し、Botを除いて名前とIDを最大25人表示する。
- 登録64件、Guild 5,000人、Action結果30秒の既存上限で処理を有限化した。
- KBC ProtocolをVersion 4へ更新し、`resolveGuildMembers`と`membersResolved`を追加した。
- Rust test 17件、Rust workspace check、通常LibraryのClippy警告ゼロ、TypeScript typecheck、Native Runtime smokeが成功した。
- Discord Developer PortalのServer Members Intentを有効化し、Local Botが正常にloginした。
- 固定Bot管理者によるメンテナー登録、GitHub Storageへの保存、`list`での現在Guild内ユーザー名表示を実Discordで確認した。
- commit `5ee9566`をmainへpushし、Northflankのbuild・deployment `success`、`/health/live` 200 alive、`/health` 200 readyを確認した。

## 残作業

1. 登録対象が固定Bot管理者ではない場合、対象ユーザー本人または対象ロール所持者で`o.push`が通ることを確認する。
2. 追加した登録対象が試験専用で不要なら末尾`del`で削除する。
3. 本環境で`o.maint maintainer list`を実行し、Localと同じ一覧が返ることを確認する。

## 補足

`cargo clippy --workspace --all-targets -- -D warnings`は、今回未変更の`motion/raster.rs`テスト内にある`chunks_exact(4)`へ、現在のRust 1.98が新しいLintを出すため失敗した。今回の変更を含む`cargo clippy --workspace --lib -- -D warnings`は成功している。この警告だけを理由にMotion実装は変更しない。
