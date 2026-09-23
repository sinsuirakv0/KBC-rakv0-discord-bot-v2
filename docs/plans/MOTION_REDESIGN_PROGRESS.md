# Motion再設計 作業チェックポイント

- 状態: Task Runtime・Motion再設計のproduction code実装済み、最小Runtime検証済み、性能・実環境確認を継続中
- 最終更新: 2026-09-23
- 設計判断: `docs/decisions/MOTION_RENDERING_V2.md`

## 確定事項

- 旧Motion実装は移植せず、仕様確認と比較だけに使う。
- TypeScriptへMotion処理を追加しない。
- Rust CoreがAsset取得からFFmpeg cleanupまで所有する。
- FrameごとのN-API往復とNode.jsへのRGBA転送を作らない。
- MP4はRustの再利用RGBA bufferからFFmpegへ直接streamし、Node.jsへFrameを渡さない。
- Motion出力は通常のbinary Actionへ載せず、Task所有の一時Fileとして送信する。
- PNG、MP4、GIFはRGBA合成後の出力経路を分ける。
- Task Runtimeはactive 1件、waiting 1件から開始する。
- `utbench`はV2へ追加しない。

## 現在までの実装

- Protocol Version 3へ更新し、Rust Coreから生成した一時Fileを送る`sendAttachmentFile`を追加した。
- Task Runtimeを追加した。active 1件、waiting 1件、初期Message送信、進捗coalescing、queue/execution/stall timeout、Action結果待ち、shutdown、workspace cleanupをRust側で管理する。
- timeout・shutdown時はCancellationを通知した後、boundedなgrace期間でJobの終了を待ってからworkspaceを削除する経路を追加した。
- `ut` / `tut`の`motion`にpng、mp4、gif、`--full`、form、frame/range指定を追加した。
- imgcut、mamodel、maanimを型付きparserで検証し、RustでMotion評価、layout、visible bounds、affine raster、RGBA合成を行う。
- PNGはRust encode、MP4はRustからRGBAをFFmpegへ直接stream、GIFは有限sample palette経路で生成する。
- `ffmpeg-static`をLocal/Node配布へ追加し、`FFMPEG_PATH`による上書きも可能にした。
- 710-fの初期代表実行で345 frame、488x390のMP4生成を確認した。Debug実行は約49.13秒、出力は1,683,615 bytesだった。
- 710-fのRelease比較を3回実行した。RGBA direct / Rust YUV420pは1.295/2.428秒、1.687/1.954秒、2.168/2.590秒で、RGBA directが全回高速だった。中央値では約30.5%短いためRGBA directを採用し、YUV production経路と`yuv`依存を削除した。
- 採用したRGBA出力を再decodeし、345 frame、488x390、30fps、11.5秒、最終pixel format `yuv420p`を確認した。
- GIFは710-f攻撃0～29の30 frameをRelease実生成し、480x398、1.00秒、504,857 bytes、再decode 30 frame、`NETSCAPE2.0` loop extensionを確認した。
- Local MP4確認で背景と旧Canvas固有の合成差を検出した。Rust Rasterizerを旧版の不透明`#252a32`背景、Multiply黒Layer、blend mode別visible boundsへ合わせた。710-fのmove 0とattack 100を旧版と同じ488x390で比較し、見た目の一致を確認した。
- 不透明背景により最終Frameのalphaが常に255になるため、毎Frameのdemultiplyと別RGBA bufferへのcopyを削除し、tiny-skiaの再利用Frame bufferをFFmpegへ直接渡すようにした。
- 最大4個のanimation asset取得を有限並列化した。
- `npm run local`はMotionを本環境に近い速度で確認できるよう、Rust Native moduleをRelease profileでbuildするようにした。
- Motion完了時にasset、prepare、encode、total、出力bytesを重複しない区間でlogへ残すようにした。
- Release Local Discordで710-fのmove・idle・attack・knockback計345 frameをMP4送信し、asset 110ms、prepare 165ms、encode 1,859ms、total 2,135ms、出力1,401,702 bytesを確認した。
- 本番DockerfileからLinux imageを構築し、Protocol v3 Native moduleと同梱FFmpeg 7.0.2の実行を確認した。Core直結Smokeでは710-fの同じ345 frameを4,009msで生成し、MP4全Frame decode、1,401,701 bytesのFile Action、Action結果後のworkspace cleanupまで確認した。
- commit `2a8ff5d`をNorthflankへ段階投入し、Deployment status `success`、`/health/live` HTTP 200 (`alive`)、`/health` HTTP 200 (`ready`)を確認した。
- 利用者が本環境Discordで代表Motionを複数回実行し、Command送信から返信まで概ね20秒と確認した。取得できた1回のlogは345 frame、asset 42ms、prepare 411ms、encode 20,656ms、total 21,110ms、出力1,405,049 bytesだった。encode区間がtotalの約97.8%を占め、Docker内encode 3,774msの約5.5倍である。旧計測86.69秒に対しては約4.1倍高速だが、RSSとqueue wait、encode区間内のRaster・pipe待ち・finalize内訳は未確認とする。

## 実装段階

### M1 Task Runtime

- [x] `sendAttachmentFile` ActionとProtocol Version更新
- [x] Rust生成pathだけを許可する契約
- [x] Attachment `actionResult`後の一時File cleanup
- [x] `TaskId`と終端状態
- [x] active 1件とwaiting 1件のQueue
- [x] busy、queue timeout、execution timeout、stall timeout
- [x] 進捗coalescing
- [x] `actionResult`と初期進捗Messageの関連付け
- [x] cancellation、child process、一時Fileのcleanup経路
- [x] shutdown統合
- [x] Task Runtimeのcancel・attachment result・cleanupを対象にした最小テスト

### M2 Motion Domain

- [x] `ut` / `tut` Motion Request
- [x] Frame・pixel・出力Size上限
- [x] Asset path解決と取得
- [x] imgcut / mamodel / maanim parser
- [x] 型付きMotion project
- [x] 2 pass評価とlayout

### M3 Rasterizer・PNG

- [x] sprite PNG decode
- [x] visible bounds前計算
- [x] SourceOver / Plus / Multiply / Screen
- [x] affine transformとopacity
- [x] 再利用RGBA buffer
- [x] PNG encode
- [x] 代表Frameの寸法・目視確認

### M4 MP4

- [x] Rust所有RGBA direct stream
- [x] RGBAとRust YUV420pの同一Release比較
- [x] 不採用YUV経路とY/U/V planeの削除
- [x] Rust所有FFmpeg child
- [x] pipe backpressureとcancel
- [x] exclusive timing計測用の経路
- [x] 710-f、345 frameで同一条件比較
- [x] 25%以上の改善Gate確認（YUV不採用、RGBA direct採用）

### M5 GIF

- [x] 有限sampleによるpalette
- [x] RGBA streaming
- [x] loop、Frame数、duration確認を含む最小smoke

### M6 Command統合・投入

- [x] `ut` / `tut`の単一検索結果からTask登録
- [x] Reaction選択後からTask登録
- [x] busy・invalid・missing・timeout表示経路
- [x] Local DiscordでのMotion実送信確認
- [x] Docker buildとFFmpeg配置確認
- [x] 数MiB出力がJSON number arrayを経由しないAction経路
- [x] 本環境へ段階投入
- [ ] 本環境の時間、RSS、Queue wait確認

## 再開位置

次回はこの順序で再開する。

1. Northflank logの`Motion generation completed`をあと2回分記録し、21,110ms前後で安定しているか確認する。
2. Northflank metricsでMotion実行時のCPUとPeak RSSを確認し、queue待機の有無も確認する。
3. encodeが支配的ならCPU割当とEncoder設定、prepareが支配的ならRasterizerを次の計測対象にする。
4. 本番ログとDiscord出力が正常なら、旧Motion経路からの移行を完了とする。

再開時に最初に読むファイルは、`docs/decisions/MOTION_RENDERING_V2.md`、`docs/decisions/TASK_RUNTIME_V1.md`、このチェックポイント、`crates/kbc-core/src/task_runtime.rs`の順とする。

`D:\KBC\KBC-rakv0-discord-bot`は読み取りだけに使用し、変更しない。
