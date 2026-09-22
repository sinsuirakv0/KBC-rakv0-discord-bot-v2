# Content Catalog実装

## 実装範囲

Phase 6では、`content/responses`と`content/help`をRust Coreの起動時に読むContent Catalogを追加した。稼働中のFile監視、live reload、Cache Frameworkは実装しない。

## Configuration

`RuntimeConfig.content_directory`がContent rootを表す。Rust単体での既定値は`content`である。

TypeScriptの`createCore()`は、build後の`apps/discord/dist/protocol`からRepositoryまたはContainer rootの`content`を絶対Pathで解決し、N-APIの`contentDirectory`へ渡す。明示値がある場合はその値を優先する。

Docker Runtime imageには`content`を`/app/content`として配置する。

## 読み込み処理

`crates/kbc-core/src/content.rs`が次を担当する。

- `ContentCatalog::load(root)`: `responses`と`help`を読み、不変のMapを作る
- `load_directory(directory)`: 対象`.txt`を最大128件まで列挙し、Command名を検証して順序を固定する
- `read_content(path)`: 1 File最大64 KiBで読み、UTF-8、BOM、改行、空本文を検証する
- `ContentCatalog::responses()`: 静的Command名と返信を決定順で列挙する
- `ContentCatalog::help(name)`: 静的Command helpとhelp indexを参照する

File名はASCII小文字、数字、`_`、`-`だけを許可する。対象外の拡張子は無視し、`.txt`で不正な名前、空本文、不正UTF-8、上限超過、Directory欠落は起動失敗にする。

## 所有権

```text
TypeScript createCore()
→ contentDirectory
→ AppRuntime::start()
→ ContentCatalog::load()
→ CommandRuntime::new(&catalog)
→ StaticResponseCommand / HelpCommandが必要なStringを所有
```

CommandへCatalog全体やFile system accessを渡さない。各Commandは必要な返信だけを所有し、実行時はChannel IDと返信から`sendMessage`を作る。

## Phase 6で実装しないもの

- HTTP Service
- Clock abstraction
- Metrics collector
- live reload
- 汎用Cache

これらは利用するCommandと観測要件が確定した時点で、必要なAPIだけを追加する。

## Phase 6検証結果

- Rust compiler、Clippy、rustfmt、TypeScript typecheckが成功した。
- Debug buildのRuntime Smokeが成功した。
- Release Native moduleへ差し替えたRuntime Smokeが成功した。
- `o.PING ignored`がFile由来の`pong!!`を返すことを確認した。
- 日本語を含む対象FileのBOMと旧Repositoryの未変更状態を監査した。
