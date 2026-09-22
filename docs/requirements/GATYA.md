# gatya Command要件

`gatya`はGuild限定Commandである。先頭のmodeは`R`（rare）、`E`（event）、`N`（normal）で、省略時はすべてを対象にする。

- `o.gatya [R|E|N]`: 開催中・近日予定のガチャを表示する。
- `o.gatya [R|E|N] <gatyaID>`: 指定ガチャの詳細をすべて表示する。
- `o.gatya [R|E|N] s<seriesID>`: seriesに属するガチャの概要を表示する。
- `o.gatya [R|E|N] <シリーズ名>`: series名を部分一致で検索する。
- 対象の後ろに`j|json`: 該当blockから`raw`を除いた整形済みJSONを表示する。
- 対象の後ろに`r|raw`: 該当blockのRaw textを表示する。
- `o.gatya help`: Content CatalogのHelpを表示する。

名称Fileが存在しないmodeでは、ガチャ名とseries mappingからseries名を補う。長文と複数blockは順序を維持して複数Messageへ分割する。取得失敗、未検出、Raw欠落の表示は旧版と同じとする。
