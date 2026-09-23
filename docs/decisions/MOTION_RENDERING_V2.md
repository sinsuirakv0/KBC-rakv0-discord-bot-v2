# Motion Rendering V2設計判断

- 状態: 採用（2026-09-23にRGBA direct streamと低CPU向けEncoder設定を確定）
- 決定日: 2026-09-22
- 対象: `ut` / `tut` Motion、Task Runtime、FFmpeg

## 状況

旧環境の代表計測では、710-f、488x390、345 frame、30 fpsのMP4生成に対し、Rust版のMotion評価は0.60秒まで短縮できた。一方で全体は73.39秒であり、FFmpeg区間は67.21秒だった。

旧計測の`RGBA transfer`は`stdout.write()`の完了待ちを含む。これはMemory copyだけでなく、FFmpegが入力を処理するまでのpipe backpressureも含むため、独立した39.36秒の処理として加算できない。`FFmpeg encoding`も描画・pipe転送と時間的に重なる。

旧経路には次の問題がある。

- Motion評価結果をFrameごとにJavaScript Objectへ変換する。
- `@napi-rs/canvas`からFrameごとにRGBAをcopyする。
- 約250.5 MiBのRGBAをNode.jsのpipeへ書き込む。
- FFmpeg側でRGBAをH.264用のYUV420pへ変換する。
- Motion専用Worker、Queue、timeout、progress、N-APIが共通Runtimeと別に存在する。
- 比較Benchmarkのhash計算がFFmpegとCPUを奪い合うため、本番相当時間と一致しない。

## 決定

旧Motion実装をV2へ移植しない。旧実装は次の用途だけに使う。

- Command syntaxと利用者向け挙動の確認
- Asset formatとMotion計算仕様の確認
- 代表素材による比較結果の作成
- Regression発生時の参照

V2では、Task登録からAsset取得、Motion評価、描画、色変換、FFmpeg、cleanupまでをRust Core内の1つのTaskとして再設計する。TypeScriptはDiscord Action実行だけを担当し、Frame、FFmpeg、Task状態を扱わない。

```text
ut / tut
→ MotionRequestを検証
→ MotionTaskRequestをTask Runtimeへ登録
→ 実行開始後にAsset取得・検証
→ PNG decodeとMotion project構築
→ Pass A: Frame評価と表示範囲計測
→ Layout確定
→ Pass B: Frame評価とRGBA合成
   ├─ PNG: Rustで1 frameをencode
   ├─ MP4: 再利用RGBA buffer → FFmpeg stdin
   └─ GIF: RGBA → palette対応FFmpeg経路
→ Task所有の一時Fileへ出力
→ pathとmetadataだけをKBC Protocolへ渡す
→ Discord AdapterがFileを送信
→ actionResult後にRustがFileを削除
```

## 互換性の境界

利用者向けの次の挙動は維持する。

- `o.ut` / `o.tut`の検索とReaction選択
- `motion png|mp4|gif`
- `a`、`w`、`i`、`k`のMotion種別
- 味方の形態指定
- Frame指定、範囲指定、複数Segmentの連結
- `--full`
- 30 fps
- Frame数、再生時間、偶数の出力寸法
- Asset不足、範囲外Frame、timeout、busyの明確な応答

新しいRasterizerとEncoderを使うため、MP4 file hashと境界画素の完全一致は互換条件にしない。欠落したpart、位置ずれ、誤った透過・合成、Frame数・時間の不一致はRegressionとして扱う。

## Rust内部の責務

初期実装は`kbc-core`内のMotion moduleとし、利用者が1つしかない段階でcrateを増やさない。

```text
motion/
├─ request       Command入力を型付きRequestへ変換
├─ assets        path解決、取得結果の検証、PNG decode
├─ project       imgcut / mamodel / maanimの型付きProject
├─ evaluate      指定FrameのScene生成
├─ layout        全Frameの表示範囲と出力寸法
├─ raster        再利用RGBA bufferへの合成
├─ encode
│  ├─ png
│  ├─ mp4
│  └─ gif
└─ job           Taskの段階、進捗、計測、cleanup
```

CommandごとのN-API関数、Worker thread、Queue、HTTP client、timeout基盤は追加しない。

## Frame処理

### 2 pass

Layout確定には対象Frame全体の範囲が必要である。全FrameのSceneをMemoryへ保持せず、次の2 passとする。

1. Motionを評価し、visible boundsだけを集約する。
2. Layout確定後に再評価し、1 frameずつ描画・encodeする。

旧計測ではRust Motion評価が0.60秒であり、全Scene保持によるMemory増加より再計算を優先する。計測で評価時間が再び支配的になった場合だけ見直す。

### Buffer

active taskは原則として次だけを再利用する。

- decoded sprite
- RGBA frame buffer 1枚
- Rasterizerが必要とする有限な作業領域

全Frame Bufferや全出力をMemoryへ蓄積しない。同じSceneが連続する場合はRGBA描画を省略し、直前Bufferを再送する。CFRのFrame数は減らさない。

### Rasterizer

最初の候補はpure Rustの`tiny-skia`とする。SourceOver、Plus、Multiply、Screen、affine sampling、透明部分の扱いを代表素材で検証してから採用を確定する。旧Canvasとの完全一致を目的にSkia全体を最初から組み込まない。

2026-09-23のLocal確認で、初期Rust実装が透明背景のまま動画化され、旧版の`#252a32`背景と異なることが判明した。旧Canvasの次の契約をRust Rasterizerへ明示的に移した。

- 各Frameを不透明な`#252a32`で初期化する。
- Multiplyは透明部分を含むcut全体を黒Layerとして合成する。
- Multiplyのlayout範囲はcut全体を使う。
- PlusとScreenのlayout範囲はalphaだけでなくRGBが非0のpixelを使う。

710-fのmove 0とattack 100を旧Canvas出力と同じ488x390で比較し、背景、位置、画角、part、色、合成結果が目視で一致することを確認した。Rasterizer差による境界のsubpixel値まではfile hash互換条件にしない。

PNG decode/encodeは専用の`png` crateを候補とする。必要な形式だけを有効にし、汎用画像Frameworkは導入しない。

## MP4経路

合成はRGB空間で行い、RGBA bufferをRust内部に1枚だけ保持する。初期案では合成後にRust内でYUV420pへ変換してFFmpegへ渡す計画だった。

- 488x390 RGBA: 761,280 bytes/frame
- 488x390 YUV420p: 285,480 bytes/frame
- 345 frame RGBA: 約250.5 MiB
- 345 frame YUV420p: 約93.9 MiB

YUV420p入力はpipe量を62.5%減らせる。一方、2026-09-23の同一Release build・同一素材・同一FFmpeg設定による3回の比較では、Rust側YUV変換を含む経路が一貫して遅かった。

| 710-f、345 frame、488x390 | RGBA direct | Rust YUV420p |
| --- | ---: | ---: |
| 1回目 | 1.295秒 | 2.428秒 |
| 2回目 | 1.687秒 | 1.954秒 |
| 3回目 | 2.168秒 | 2.590秒 |
| 中央値 | 1.687秒 | 2.428秒 |

RGBA directは中央値で約30.5%短かった。YUV経路は「25%以上短縮」の採用Gateを満たさず、逆に遅くなった。FFmpeg内の色変換が十分高速であり、この環境ではRust側の色変換Costがpipe削減効果を上回ったと判断する。

本番経路はRustの再利用RGBA bufferからFFmpeg stdinへ直接streamし、FFmpeg出力だけを`yuv420p`にする。Node.js、N-API、JSONへFrameを渡さない点は維持する。全Frameは保持せずactive taskも1件なので、pipe byte数の増加はMemory bufferの無制限増加を意味しない。`yuv` crateとY/U/V planeはproduction codeから削除する。

FFmpegは初期段階では外部ProcessのままRustが所有する。libavcodecやx264を直接linkせず、Process分離、kill、配布の単純さを優先する。thread数は1から開始する。

### 低CPU環境向けEncoder設定

Northflank本番の代表実行では、345 frameの`encode_ms`が20,656 msとなり、全体21,110 msの約97.8%を占めた。従来の`encode_ms`にはFrame評価・描画、blocking worker待ち、FFmpeg stdinのbackpressure、Encoder終了待ちが含まれていたため、MP4経路に次の累積計測を追加した。

- `encoder_total_ms`
- `frame_worker_ms`
- `frame_render_ms`
- `spawn_overhead_ms`
- `encoder_write_ms`
- `encoder_finalize_ms`
- `rendered_frames` / `reused_frames`
- `input_rgba_bytes`
- `queue_wait_ms`

FFmpegの`-threads 1`は入力より前ではなく、libx264の出力Optionとして`-threads:v 1`を指定する。色変換Filterも`-filter_threads 1`に制限する。CPU 0.2相当のDocker制限下で710-fを3回ずつ比較した結果は次のとおりだった。

| 710-f、345 frame、488x390 | encode中央値 | total中央値 | 出力bytes |
| --- | ---: | ---: | ---: |
| 変更前 | 22,091 ms | 22,516 ms | 1,401,701 |
| 明示的1 thread | 16,897 ms | 17,283 ms | 1,405,036 |
| FFmpeg auto thread | 24,505 ms | 24,871 ms | 1,401,701 |

明示的1 threadは変更前よりencode中央値を約23.5%、auto threadより約31.1%短縮したため採用する。MP4は345 frame、488x390、30 fps、11.5秒、`yuv420p`を維持した。変更前出力との復号後SSIMは0.998678であり、利用者向けのFrame・時間・寸法・見た目の互換条件を満たす。Encoder thread数によるH.264 bitstreamと境界画素の完全一致は互換条件にしない。

Frameごとの進捗文字列生成は16 frameごとに間引く。Task Runtime側のDiscord編集は従来どおり2秒間隔でcoalesceされるため、表示頻度は変えずHot loop内のAllocationだけを減らす。

Frameごとの`spawn_blocking`除去も同条件で3回比較した。描画worker区間は短くなったが、encode中央値は16,897 msから17,396 msへ約3.0%悪化した。低CPU環境ではRust描画とFFmpegのSchedulingへ影響するため、現行のblocking workerを維持する。全Frameを保持するpipelineや追加Channelも導入しない。

代表素材の再利用Frameは345 frame中2 frame（約0.58%）だった。CFRを崩すVFR化、DrawPacket全Frame cache、完成MP4 cacheはこの改善では導入しない。現在の支配区間は`encoder_write_ms`であり、低い再利用率に対してMemory、Invalidation、複雑性を増やす根拠がない。

## PNGとGIF

- PNGは指定Frameだけを評価・描画し、Rustから直接encodeする。FFmpegを起動しない。
- GIFはpalette生成の性質がMP4と異なるため、RGBAの有限なsampleからpaletteを作り、全Frameはstreamする。
- GIFの高速化はMP4経路の成立後に計測し、独自quantizerは必要性が確認できるまで導入しない。

## 大容量Binary境界

既存の`sendAttachment`はRustの`Vec<u8>`を`serde_json::Value`へ変換し、JavaScript側で`Buffer.from(action.data)`している。小さな画像・Asset添付には維持するが、MotionのPNG・MP4・GIFには使用しない。

Motionでは新しい`sendAttachmentFile` ActionをKBC Protocolへ追加する。

```text
Rust Task
→ Task専用一時directory内へ出力
→ sendAttachmentFile(path, fileName, contentType, message)
→ N-APIは小さな文字列metadataだけを変換
→ discord.jsがlocal fileを送信
→ actionResult
→ Task RuntimeがFileとdirectoryを削除
```

- pathはRustが作成したTask専用directory内のFileだけを許可する。
- User入力、Remote data、Command引数をpathとして渡さない。
- TypeScriptはpathの意味を判断せず、Discord添付へ変換する。
- 送信成功・失敗・timeout・shutdownのすべてでRustがcleanupする。
- Discord送信が完了する前にFileを削除しない。
- 新Actionを理解できない旧Adapterとの混在を防ぐため、Protocol Versionを上げる。

Native Buffer専用N-APIを別経路で追加する案は、Protocol境界が二重化するため初期案では採用しない。local file経路でMemoryと速度を計測し、不足が確認された場合だけ再検討する。

## Task Runtimeとの接続

Task RuntimeはMotion専用実装にせず、最初の利用者としてMotionを載せる。

- active task: 1件
- waiting queue: 1件
- QueueにはAsset byte列ではなく、検証済みRequestとAsset識別子だけを保持
- Queue満杯: 即時busy
- queue timeout: 10分
- execution timeout: 10分
- stall timeout: 60秒
- progress編集の最短間隔: 2秒
- 終端: succeeded / failed / timed-out / cancelledのいずれか1回

Task Runtimeは初期Message、進捗Message ID、FFmpeg child、一時directory、出力File Action、終了状態を所有する。初期Messageの`actionResult`が失敗したTaskは重い処理を開始しない。Attachment送信の`actionResult`を受け取るまでTaskを終端せず、結果確定後に出力Fileを削除する。

shutdown・timeout時は次の順序を守る。

```text
Cancellation通知
→ Frame loop停止
→ FFmpeg stdinを閉じる
→ FFmpegを有限時間待つ
→ 必要ならkill
→ child終了を回収
→ 一時File削除
→ slot解放
```

cleanup完了前に次Taskへslotを渡さない。

## 有限な上限

初期値は次とし、本番計測後にだけ変更する。

- MP4/GIF: 最大900 frame（30秒）
- PNG: 1 frame
- Video pixel数: 最大480x400相当
- 圧縮済みAttachment: 最大8 MiB
- FFmpeg stderr保持: 末尾8 KiB

入力Assetは共有`HttpService`の既存Response上限を使う。上限超過時に自動で品質を落とさず、利用者へ範囲短縮を案内する。

## 計測

Task全体の段階は重複しない区間として記録し、MP4 Encoder内部は総時間と内訳を併記する。

- queue wait
- asset load
- parse/project
- bounds pass
- raster
- blocking worker待ち
- encoder write blocked
- encoder finalize
- attachment upload
- total execution
- peak RSS
- rendered frame / reused frame
- input bytes / output bytes

`encoder_total_ms`はMP4 Encoder全体のwall timeであり、`frame_worker_ms`、`encoder_write_ms`、`encoder_finalize_ms`を内包する。`frame_worker_ms`は`frame_render_ms`を内包し、差分を`spawn_overhead_ms`として記録する。Frameごとのlogは出さず、完了時の累積値だけを出力する。

比較hashは通常Benchmarkへ含めない。必要なCompatibility確認では計測本体と分離する。

最初の性能Gateは、同じbuild、同じ素材、同じCPU割当でRGBA入力経路とYUV420p入力経路を比較する。2026-09-23の比較でYUV経路はGateを満たさなかったため、RGBA directを既定経路として採用した。2倍高速化は目標とするが、完了条件にはしない。

## 最小検証

恒久テストは次に限定する。

- Motion引数と安全上限のpure logic
- Asset parserの代表fixture
- Rasterizerの4合成Modeを含む少数の代表Frame
- Taskの終端が1回だけになることとcleanup smoke

旧実装との全Frame比較、encoder比較、性能Benchmarkは移行中だけの検証とし、通常Test Suiteへ残さない。

代表素材には少なくとも次を含める。

- 710-fの345 frame MP4
- shared assetを使う味方
- 敵Motion
- Plus / Multiply / Screenを含む素材
- PNG、MP4、GIF

## 採用しない案

- 旧JavaScript Motion engineの移植
- FrameごとのN-API往復
- Node.js WorkerからRGBAをpipeする構成
- 全Frame RGBAのMemory保持
- Rust側でのYUV420p事前変換
- YUV空間での直接alpha合成
- 最初からSkia全体またはlibavcodecをlinkする構成
- 重複FrameをVFR化してFrame数を変える最適化
- 全FrameのDrawPacket cacheと完成Motion file cache（現在の支配区間・再利用率に対して根拠不足）
- FFmpeg thread数のauto設定（CPU 0.2相当で明示的1 threadより遅い）
- Frameごとの`spawn_blocking`除去（end-to-end中央値が改善しない）
- Motion専用Queue、HTTP client、timeout、progress基盤

## 実装順序

1. `sendAttachmentFile`のProtocol契約と一時File所有権を確定する。
2. Task Runtimeの内部契約と有限Queueを実装する。
3. Motion Request、Asset、Project、2 pass Layoutを実装する。
4. Rust Rasterizer spikeで4合成Modeを確認する。
5. PNG経路で座標・透過・layoutを確認する。
6. RGBA入力のMP4参照経路を一時的に用意する。
7. YUV420p経路を実装して同一条件で比較し、RGBA directを採用する。
8. YUV420p経路を削除し、GIFを追加する。
9. Local Discordで確認後、本環境へ段階投入する。

旧実装のQueue、Worker、Canvas Adapter、Benchmark CommandはV2へ追加しない。
