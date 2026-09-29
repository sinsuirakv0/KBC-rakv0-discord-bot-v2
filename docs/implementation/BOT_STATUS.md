# Bot status実装

## 構成

```text
commands/botstatus/mod.rs         Command、Session、Scheduler、Formatter、累計稼働時間
commands/botstatus/data_source.rs Northflank API、検証、Cache、Metric集計
commands/botstatus/graph.rs       CPU・Memory PNG描画
commands/botstatus/model.rs       取得Snapshotと表示範囲
storage.rs                        state/bot-uptime.json
```

`BotStatusCommand`は初回Message作成だけを`BotStatusService`へ依頼する。Serviceは`TaskRuntime::request_adapter`でMessage IDを取得し、そのIDをKeyに有限Sessionを保持する。Background workerは1秒ごとに期限だけを確認し、取得が必要な15秒周期にだけNorthflankへアクセスする。

## Northflank取得

- `GET /v1/projects/{projectId}/services/{serviceId}`: Service名、Plan、Instance、状態
- `GET .../deployments`: 現在CommitとV2初回Deployment
- `GET .../containers`: Running数と現在稼働時間
- `GET .../metrics`: CPU、Memory、Network、Request、HTTP 5xx

MetricはContainerごとの同一timestamp値をCPU・Memoryでは平均、Network・Request・5xxでは合計する。APIの`metricUnit`をそのまま表示単位へ変換する。

Metadata Cacheは60秒、30分・1時間Metric Cacheは個別に12秒保持する。Cache missの取得は共有更新Lock内で行うため、複数Sessionが同時に期限へ達しても連続した重複取得を抑える。

## Message更新

初回は`sendRichMessage`、Button直後は`editRichInteractionReply`、自動更新は`editRichMessage`を使用する。新しいGraphを添付する編集では既存添付を置き換え、停止表示だけの編集では直前のGraphを保持する。

InteractionはAdapterで`deferUpdate()`してからRegistryへ最大64件・15分だけ保持する。Service側SessionとAdapter側参照は別責務であり、前者は状態・期限、後者はDiscord応答Transportだけを持つ。

## 累計稼働時間File

```json
{
  "schemaVersion": 1,
  "baselineAt": "2026-09-22T03:07:19Z",
  "accountedThrough": "2026-09-30T00:00:00Z",
  "accumulatedSeconds": 670361,
  "baselineEstimated": false
}
```

`baselineAt`は初回V2 Deployment、`accountedThrough`は加算済みの時刻、`accumulatedSeconds`は累計秒数である。既存Fileとの競合時は最新版を再読込し、最新版の加算済み時刻と現在processの起動時刻より後の区間だけを追加する。

## 参照API

- https://northflank.com/docs/v1/api/project/services/get-service
- https://northflank.com/docs/v1/api/project/services/get-service-metrics
- https://northflank.com/docs/v1/api/project/services/list-service-containers
- https://northflank.com/docs/v1/api/project/services/list-service-deployments
