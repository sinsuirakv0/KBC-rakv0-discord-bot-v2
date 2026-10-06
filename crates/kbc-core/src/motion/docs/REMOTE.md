# LINE向けMotion代行API

2026-10-07。実装済み、ローカルHTTPで結合検証済み。LINE側の生成失敗時にDiscord BotのRendererを使い、完成ファイルだけを返す。Discord・LINEへの投稿はこのAPIでは行わない。

## 起動と契約

`MOTION_RENDER_SECRET`を32〜256文字の印字可能ASCIIで指定すると有効になる。通知用HTTPサーバーと`EVENT_UPDATE_PORT`（未設定時`PORT`または3000）・`EVENT_UPDATE_HOST`を共有する。通知用SecretとGitHub設定はmotion単独起動には不要。公開接続はHTTPS終端を使う。全motion要求は`Authorization: Bearer <secret>`で認証する。

| HTTP | 入出力 |
| --- | --- |
| `POST /motion-jobs` | 最大16KiBのJSON。`protocolVersion=1`、40桁hexの`requestId`、`assetRevision`（mainまたは40桁commit）、`plan`を渡す。受付は202 |
| `GET /motion-jobs/:id` | JSONの`protocolVersion=1`と`status=pending / ready / failed`。存在しなければ404 |
| `GET /motion-jobs/:id/artifact` | 完成したPNG / GIF / MP4のbyte。`Content-Type`、`X-Motion-Protocol-Version=1`、`X-Motion-File-Name`、MP4だけ`X-Motion-Duration-Ms`を返す |
| `DELETE /motion-jobs/:id` | 生成取消または成果削除。存在しなくても200 |

`plan`はsnake_caseの`MotionPlan`で、形式は`Png / Gif / Mp4`、motionは`Attack / Move / Idle / Knockback`。素材path・形式・segment・full・preview_scale・filename_stemをLINE側の解決結果から受け取る。受信側でもspriteを`Number/<base>.png`、cut/model/animationを同じbaseの`ImageData/`へ限定し、名前・数値・segment数・900Frameの上限を検証する。任意URLや任意のローカルpathは受け付けない。

同じID・同じ本文は既存の依頼を返す。同じIDで内容が違えば409、容量超過は429、未認証は401、不正な入力は400。認証は長さ確認とtimingSafeEqualを使う。HTTPでの同時処理は最大4件、各15秒で、ファイル書込完了まで枠を保持する。

## 関数と資源

| 関数・型 | 働き・関係 |
| --- | --- |
| `createMotionHandler` | 認証・HTTP形式・本文上限・配送を担当。状態は持たずNativeへ渡す |
| `AppRuntime::{submit_motion,motion_status,read_motion_artifact,remove_motion}` | 外部入力をRemoteMotionServiceへ接続する |
| `RemoteMotionService::submit` / `validate` | 型・有限条件・ID一致・結果保持枠を検査し、共通HttpServiceを使うMotionJobを既存TaskRuntimeへ渡す |
| `Record::refresh` / `RemoteMotionService::prune` | oneshot完了を成果・失敗へ変え、取消済み・完了10分後の作業領域を次のAPI呼出時に削除する |
| `RemoteMotionService::read / remove / shutdown` | 最大8MiBのBuffer取得、生成取消・成果削除、停止時の掃除 |
| `MotionJob::render` / `finish_encoder` | 既存描画とFFmpegを使い、実Frame数からMP4のdurationを計算。取消時は子プロセスの終了を待つ |

待機・生成は[共通TaskRuntime](../../../docs/TASK_RUNTIME.md)の受付2件・描画1件と期限を共有する。外部依頼・完成・失敗の保持は合わせて2件、完成ファイルは最大合計16MiB。完了後はTask受付枠を解放するが、受取・削除がない成果が2件あれば新しい代行は429にする。通常のDiscord Taskはこの成果保持枠を使わない。

結果はプロセス内の有限Mapと一時ファイルであり、再起動では失われる。LINE側が同じIDで再受付した場合は再生成できる。素材参照mainの内容は生成時点に依存するため、固定commitを渡さない場合のbyte一致は保証しない。既存Event / ActionのProtocol v7とは独立した代行Protocol v1を使い、NativeとAdapterを同じ変更で配備する。

検証手順はLINE側`experiments/motion-fallback/run.mjs`、結果は同repoの`experiments/motion-fallback/docs/VERIFICATION.md`。両Botをbuildした後、LINE repoから実行する。LINE / Discord認証を使わず、ローカルHTTPと公開ゲーム素材だけを使う。本番への公開・実LINE送信・低資源コンテナの負荷確認は別の運用検証になる。
