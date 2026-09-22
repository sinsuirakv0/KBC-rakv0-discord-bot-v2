# Content由来Command登録

## Context

現行Botは`content/responses/*.txt`のFile名をCommand名として静的Commandを登録する。Phase 5の`CommandMetadata`は`&'static str`を前提としており、Content由来の名前を扱うにはCommand名のhardcodeかMetadata APIの変更が必要である。

また、旧`o.help`のindexにはV2へ未移植のCommandも記載されている。移行途中だけContentを書き換えると正本との差分が生まれ、後で戻すMigration Codeが必要になる。

## Decision

- `CommandMetadata`はCommandが所有する`String`のnameと`Vec<String>`のaliasesを持つ。
- 起動時にContent Catalogの全Responseから静的Commandを構築する。
- 静的Commandは通常Responseとhelpを所有し、第一引数が`help`の場合だけhelpを返す。
- help欠落時は現行どおり通常Responseへfallbackする。
- `help` Commandはindex Contentを所有し、引数を無視する。
- 旧help indexは変更しない。Phase 7の本番機能一覧としては扱わず、段階移行中の互換Contentとして扱う。

## Consequences

- 静的Command追加時はResponse Fileを追加するだけでRegistryへ反映できる。
- Metadataごとに小さな所有Allocationが発生するが、起動時だけでありCommand実行のHot Pathではない。
- Command Metadataを`static`定数にする必要がなくなり、今後の設定由来aliasにも対応できる。
- Phase 7中の`o.help`は未実装Commandも表示するため、V2を本番へ切り替える前に移植状況との整合を再確認する必要がある。
