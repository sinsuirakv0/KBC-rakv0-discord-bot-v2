# sale Command要件

`sale`はGuild限定Commandである。

- `o.sale`: 終了していない常設以外のsale eventを、開催中・予定に分けて表示する。
- `o.sale <ID>`: 指定stage IDを含むentryの詳細をすべて表示する。missionはID検索だけを行う。
- `o.sale <名前>`: sale名と終日event名を部分一致で検索する。
- `o.sale <ID> j|json`: 該当entryから`raw`を除いた整形済みJSONを表示する。
- `o.sale <ID> r|raw`: 該当entryのRaw textを表示する。
- `o.sale help`: Content CatalogのHelpを表示する。

検索結果の順序と初期一覧は旧版を維持する。9件以下の検索結果へReactionを付けて選択後の詳細を返す操作は、Phase 9のSession Managerで移植する。それまでは一覧表示で終了する。
