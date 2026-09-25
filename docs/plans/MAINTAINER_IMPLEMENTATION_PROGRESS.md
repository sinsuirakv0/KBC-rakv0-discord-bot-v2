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
- `list`の閲覧権限を固定Bot管理者だけから、固定Bot管理者または登録済みメンテナーへ変更した。追加・削除は固定Bot管理者限定のまま維持する。

## 残作業

1. 登録対象が固定Bot管理者ではない場合、対象ユーザー本人または対象ロール所持者で`o.push`が通ることを確認する。
2. 追加した登録対象が試験専用で不要なら末尾`del`で削除する。
3. 本環境で登録済みメンテナーから`o.maint maintainer list`を実行し、一覧が返ることを確認する。

## 補足

`cargo clippy --workspace --all-targets -- -D warnings`は、今回未変更の`motion/raster.rs`テスト内にある`chunks_exact(4)`へ、現在のRust 1.98が新しいLintを出すため失敗した。今回の変更を含む`cargo clippy --workspace --lib -- -D warnings`は成功している。この警告だけを理由にMotion実装は変更しない。

## 通知ロール拡張（2026-09-25）

- 状態: 実装・自動検証完了、実Discord確認前
- `o.maint role`、Guildごと最大9件の`pushsetting`選択肢、単一の永続Reactionパネルを実装した。
- Reaction追加・解除を権限ゼロかつBot管理可能なRoleの付与・解除へ接続した。
- `o.push <category> role:<ID> [del]`で、登録済み通知先へ複数Role mentionを設定できる。通知先は自動作成しない。
- `o.push skd url <URL> add|del`で、Guild固有の関連サイトを`o.skd`とSKD通知へ追加できる。
- `o.skd`は通常の`sendMessage`だけを使い、通知Role mentionを許可しない。
- Protocol Version 5、Rust test 21件、Rust workspace check、Library Clippy、Rustfmt、TypeScript typecheck、Native Runtime smokeが成功した。

### 実Discord確認順

1. `o.maint role <試験名>`で権限ゼロのRoleが作られることを確認する。
2. `o.maint pushsetting <Role ID> add`後、`o.maint pushsetting`で番号Reaction付きパネルを設置する。
3. 一般ユーザーのReaction追加・解除でRoleが付与・解除されることを確認する。
4. `o.push skd`登録済みChannelで`o.push skd role:<Role ID>`を実行し、次回更新通知だけがRoleをmentionすることを確認する。
5. `o.push skd url <URL> add`後に`o.skd`を実行し、「関連サイト」にURLが増える一方でRole mentionが発生しないことを確認する。
6. パネルを別Channelへ移し、旧MessageのReaction消去と「通知設定は移動しました」への編集を確認する。
