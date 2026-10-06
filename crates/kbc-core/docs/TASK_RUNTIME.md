# 有限なTask実行と外部成果

2026-10-07。`task_runtime.rs`は通常のDiscord Taskと外部生成Taskを同じ受付2件・描画1件の枠で実行する。外部生成のHTTP契約・結果保持は[Motion代行](../src/motion/docs/REMOTE.md)を参照。

| 関数・型 | 働き・関係 |
| --- | --- |
| `TaskRuntime::submit` / `TaskSubmission` | 通常Taskを共通Queueへ登録。Discordで受付・進捗・完成を投稿する |
| `TaskRuntime::submit_external` | 同じ受付枠を取得し、成果をoneshotで返す。Discordへの投稿を作らない |
| `run_scheduler` / `QueuedSubmission` | 最大2件を管理し、Discordと外部の完了経路だけを分ける |
| `run_discord_task` / `run_external_task` | 共通描画枠を取得し、TaskContextの作業領域・取消・待機時間を渡す |
| `supervise_job` | 実行10分、無進捗60秒、停止・取消を管理。外部Taskでも進捗を監視し、Discord向け進捗送信だけを省く |
| `cancel_and_wait` | 取消後15秒の終了を待つ。終了しなければ共通描画枠をcloseし、新しい生成を拒否する |
| `TaskArtifact` / `cleanup_task_workspace` | 完成ファイル・名前・形式・任意durationを保持。通常は送信結果後、外部は受取・期限終了後に削除 |

待機上限は10分。外部Taskの待機は停止・依頼取消でも解除する。出力は作業領域直下の通常ファイルで最大8MiB。Taskのpanicも既存JoinSetで回収し、外部の結果経路切断を失敗へ変える。

取消で描画枠を閉じた場合、軽量Commandは継続できるが生成の再開にはBot再起動が必要。実処理が残っている可能性があるため、新しい描画で置き換えない。外部結果を保持する追加枠は2件で、完成したTaskの実行枠は解放する。
