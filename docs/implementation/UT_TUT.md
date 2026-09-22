# `ut` / `tut` 非Motion実装

## 対象

Rust Coreへ次を移植する。

- 名前・別称・ID検索
- 1〜3件の詳細URL返信
- 4〜9件の数字Reaction選択
- 10〜20件の一覧
- 21件以上の20件単位ページ操作
- `origin`画像添付
- 実在Assetだけを対象にした`file`選択と添付

Motionは旧実装を移植せず、`docs/decisions/MOTION_RENDERING_V2.md`の新設計をTask Runtimeと同時に追加する。現時点の`motion`指定は既存の不正指定Messageを返す。

## 構成

```text
commands/ut, commands/tut
├─ mod.rs          Parse、検索、整形、Session Continuation、Asset path解決
└─ data_source.rs  Remote data検証と共有Service接続

commands/common
├─ search.rs       NFKC、ひらがな化、長音・波線の検索正規化
├─ file_picker.rs  9件単位のfileページとReaction継続
└─ remote_data.rs  検証済みsnapshotと有限なAsset存在Cache
```

Commandは`HttpService`を直接構築しない。Text metadataは10分TTLで、ETagとLast-Modifiedを使って再検証する。更新またはParseに失敗した場合、過去に検証成功したsnapshotだけを利用する。同時refreshはresource単位のMutexで1本にまとめる。

Asset存在確認は共有HTTPのHEADを使い、1 batch最大6件、Cache最大512件とする。最古の確認結果から退避し、添付file本体はCacheしない。

## Session

- 検索候補選択: 最大9 Reaction
- 一覧ページ: 前・次の最大2 Reaction
- file選択: 数字9件と前・次の最大11 Reaction
- TTL: Reaction設定完了後60秒
- owner以外のReactionは無視
- 有効Reaction後は全Reactionを削除
- ページ移動時は同じMessageを編集して再武装
- timeout時は全Reactionを削除し、受付終了footerへ編集

検索候補からfile操作へ進む場合、候補Messageを「選択済み」へ編集した後、新しいfile prompt Messageを送る。その`sendMessage`結果を次のSessionへ結び付ける。

## Data

`tut`は`Enemyname.tsv`と`enemyname.json`を検証後に結合する。表示名が`ダミー`の場合は一致別称を表示へ利用する。

`ut`は`character-index.json`のID連番、1〜4形態、別称重複を検証する。`unitbuy.csv`は63列以上と共有形態IDを検証し、共有Assetの`m` suffixをorigin・fileの両方で解決する。

## 確認済み経路

Core直接Smokeで次を確認した。

- `o.tut 0`、`o.ut 0`の詳細URL
- `o.tut 0 origin`、`o.ut 0 origin`のPNG添付
- `o.tut 0 file`、`o.ut 0 file f`の実在候補
- `ut file`のReaction選択、Message編集、添付
- `o.ut ネコ`の1ページ目から2ページ目への再武装
