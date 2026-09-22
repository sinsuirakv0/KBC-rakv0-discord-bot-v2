# Content Catalog V1

## Context

現行Botは`content/responses`と`content/help`をLocal Fileから読む。Phase 5の`ping`はRuntime経路を先に検証するため返信をhardcodeしたが、今後の静的Commandとhelpを移す前にContentの所有権と失敗Policyを決める必要がある。

HTTP、Clock、MetricsもPhase 6の候補だが、現時点のCommandはそれらを利用しない。未使用のService APIを先に固定すると、実用Commandの要件と合わない抽象化になる可能性がある。

## Decision

- `RuntimeConfig`にContent root directoryを持たせる。
- `AppRuntime::start()`で`responses`と`help`を同期的に読み、不変のCatalogを作る。
- 起動後のFile I/Oとlive reloadは行わない。
- CommandへCatalog全体を渡さず、構築時に必要なContentだけを注入する。
- `.txt`だけを対象とし、Command名に利用可能なASCII小文字・数字・`_`・`-`だけをFile名として許可する。
- Directoryごとの対象File数と1 FileのByte数を有限にする。
- BOMと改行を正規化し、Repositoryで標準的な末尾改行を1つ除く。本文の表示内容は現行と同じにする。
- 必須Contentの欠落や不正Contentは起動失敗とする。

## Consequences

- 稼働中のCommand実行でFile I/Oが発生せず、途中でContentが変化しない。
- 配置不備を最初のCommand実行まで遅延させず、起動時に検出できる。
- Content変更にはProcess再起動が必要になる。
- HTTP・Clock・Metricsは今回実装せず、利用者が現れた時点で必要なAPIだけを設計する。
