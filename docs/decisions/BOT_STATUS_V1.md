# Bot Status V1

## 状態

採用。

## 判断

`o.botstatus`の取得・集計・Session・稼働時間管理はRust Coreの`BotStatusService`へ置く。TypeScript Discord AdapterはProtocolで渡されたEmbed、添付、ButtonをDiscord objectへ変換し、InteractionをdeferしてCoreへ返すだけとする。

Northflank取得には共有`HttpService`を使用する。Service・最新Deployment・Containerは60秒、Metricは表示範囲ごとに12秒Cacheする。複数画面はCacheと更新Lockを共有する。30分・1時間の両方が継続利用されても定常時は既定のNorthflank API上限1,000回/時を超えない設計とする。

```text
o.botstatus
→ Northflank Service / Deployment / Containers / Metrics
→ Rustで集計・PNG生成
→ sendRichMessage
→ 15秒Scheduler
→ editRichMessage
```

## Protocol

Protocol Version 7で汎用`RichMessage`と次のActionを追加する。

- `sendRichMessage`
- `editRichInteractionReply`
- `editRichMessage`

`RichMessage`は本文、Embed、Message Component、byte attachmentを持つ。`componentInteraction`には対象MessageをSessionへ結び付ける`messageId`を追加する。Bot status固有のDiscord Actionは作らない。

## Sessionと負荷上限

- 画面Sessionは最大8件。
- Bot status Interaction処理は最大4件。
- 自動更新は15秒間隔、無操作停止は60秒、停止後保持は15分。
- 更新処理は1本のLockで直列化し、同じCacheを利用する。
- PNGは固定1,000×480 pixelとし、CPUとMemoryの2 panelだけを描く。

custom IDへ所有者IDを含め、Adapterで他ユーザーへephemeral拒否を返す。Coreでも所有者、Channel、Message IDを再検証し、Adapterだけを認可根拠にしない。

## 累計稼働時間

初回値はV2初回Deploymentから機能初期化までの経過時間とする。その後はprocess起動時刻以降だけを加算し、1時間ごと、画面停止時、正常終了時に既存GitHub Storageへ保存する。

同じServiceの旧・新processが同時に保存する可能性があるため、保存済みの`accountedThrough`と各processの起動時刻の遅い方から追加分を計算する。これによりrolling deploymentの重なりを二重加算せず、停止期間も加算しない。

## トレードオフ

- 画像へ文字を描かず、系列名と数値はEmbed fieldへ置く。Font追加と描画負荷を避けられる一方、画像単体では系列名が分からない。
- GitHub Storage未設定時はprocess memoryだけの累計となり、再起動で初期化される。
- 強制終了では最後のCheckpoint以降を失う可能性がある。1時間Checkpointで損失を有限にし、常時GitHubへ書き込む負荷は避ける。
- API取得に失敗した更新は現在のMessageを維持し、次の手動・自動更新で回復を試みる。
