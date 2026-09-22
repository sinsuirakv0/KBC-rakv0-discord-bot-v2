# item Command要件

`item`はGuild限定Commandである。

- `o.item`: 終了していない常設以外のアイテム配布を、開催中・予定に分けて表示する。
- `o.item <ID>`: まず`giftType`、見つからなければ`eventID`で検索して詳細をすべて表示する。
- `o.item <ID> j|json`: 該当entryから`raw`を除いた整形済みJSONを表示する。
- `o.item <ID> r|raw`: 該当entryのRaw textを表示する。
- `o.item help`: Content CatalogのHelpを表示する。

`eventID`へfallbackした場合は、その旨を最初に通知する。長文と複数entryは順序を維持して複数Messageへ分割する。取得失敗、未検出、Raw欠落、使い方の各表示は旧版と同じとする。
