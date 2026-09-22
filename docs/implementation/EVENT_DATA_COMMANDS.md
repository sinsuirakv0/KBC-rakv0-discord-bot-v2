# 外部Data Command実装

## 対象

Rust Coreへ`item`、`sale`、`gatya`を移植した。TypeScript Discord AdapterにはCommand固有処理を追加していない。

```text
messageCreate
→ CommandRuntime
→ item / sale / gatya Command
→ Command固有DataSource
→ 共有HttpService
→ parse・validate・format
→ CommandOutput（最大32 Action）
→ Action Queue
```

## 共通部品

- `services/http.rs`: Client再利用、同時実行permit、timeout、status、Response size、UTF-8を検証する。
- `services/clock.rs`: 現在時刻をCommandへ注入し、DiscordやHTTPから時刻取得を分離する。
- `commands/common/schedule.rs`: event header日時の検証、JST表示、曜日、期間、time blockを共有する。
- `commands/common/output.rs`: Discord messageをUTF-16 1800単位で分割し、必要ならcode fenceを付けて`CommandOutput`へ追加する。

## Commandの分割

各Commandは必要な範囲だけ`mod.rs`、`data_source.rs`、`model.rs`へ分ける。

- `mod.rs`: 引数解釈、検索、表示整形、出力順序
- `data_source.rs`: URL、並列取得、JSON・CSV・TSVのparseと入力検証
- `model.rs`: Remote JSONとCommand内dataの型

DataSourceは共有`HttpService`だけを受け取る。CommandはDataSourceと`Clock`を受け取り、Runtime全体やContent Catalogを参照しない。

## Error

取得・parse・data検証の詳細は標準Errorへ記録し、Discordには旧版互換の`❌ データ取得に失敗しました`だけを返す。HTTP 404を許容するのは、`gatya`の任意名称Fileだけである。

出力が32 Actionを超えた場合はCommand実行失敗とし、Runtimeが`❌ コマンドの実行に失敗しました`へ置き換える。

## 現在の制限

`sale <文字列>`は旧版と同じ検索結果一覧を返す。結果が9件以下の場合にReactionを付け、30秒待ち、選択結果の詳細を返す部分は未実装である。この処理は`reactionAdd`を跨ぐため、Phase 9のSession Managerへ移す。

Cacheはない。1回のCommand内では`tokio::try_join!`で必要Fileを並行取得するが、Bot全体のHTTP同時実行数は共有Semaphoreで制限される。
