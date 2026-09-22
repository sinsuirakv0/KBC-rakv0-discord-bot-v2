//! Motion TaskのAsset取得、2-pass描画、Encoder、出力File作成を接続する。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use png::{BitDepth, ColorType, Compression, Encoder, Filter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::task::spawn_blocking;

use crate::commands::common::remote_data::RemoteAssetSource;
use crate::task_runtime::{TaskArtifact, TaskContext, TaskFailure, TaskFuture, TaskJob};

use super::MotionError;
use super::MotionPlan;
use super::assets::load_motion_assets;
use super::evaluate::{DrawPacket, evaluate};
use super::layout::{FrameRef, measure, resolve_frames};
use super::project::MotionProject;
use super::raster::{Rasterizer, SpriteSheet};
use super::request::MotionFormat;

const FRAME_RATE: u32 = 30;
const STDERR_LIMIT: usize = 8 * 1024;
const PALETTE_SAMPLE_COUNT: usize = 16;

pub(crate) struct MotionJob {
    plan: MotionPlan,
    assets: RemoteAssetSource,
    ffmpeg_path: Option<PathBuf>,
}

impl MotionJob {
    pub(crate) fn new(
        plan: MotionPlan,
        assets: RemoteAssetSource,
        ffmpeg_path: Option<PathBuf>,
    ) -> Self {
        Self {
            plan,
            assets,
            ffmpeg_path,
        }
    }
}

impl TaskJob for MotionJob {
    fn run(self: Box<Self>, context: TaskContext) -> TaskFuture {
        Box::pin(async move {
            self.render(context)
                .await
                .map_err(|error| TaskFailure::new(error.public_message(), error.to_string()))
        })
    }
}

impl MotionJob {
    async fn render(self, context: TaskContext) -> Result<TaskArtifact, MotionError> {
        let total_started = Instant::now();
        context.report("⏳ モーション素材を取得しています");
        let asset_started = Instant::now();
        let assets = load_motion_assets(&self.plan, &self.assets)
            .await
            .map_err(|error| MotionError::unavailable(error.to_string()))?;
        let asset_elapsed = asset_started.elapsed();
        let plan = self.plan.clone();
        let prepare_context = context.clone();
        let prepare_started = Instant::now();
        let mut prepared =
            spawn_blocking(move || PreparedMotion::new(plan, assets, &prepare_context))
                .await
                .map_err(|error| MotionError::render(format!("motion worker failed: {error}")))??;
        let prepare_elapsed = prepare_started.elapsed();
        let frame_count = prepared.frames.len();
        let dimensions = prepared.dimensions();
        let extension = self.plan.format.extension();
        let output = context
            .output_path(&format!("motion.{extension}"))
            .map_err(|error| MotionError::render(error.to_string()))?;
        let encode_started = Instant::now();
        match self.plan.format {
            MotionFormat::Png => {
                let output_for_worker = output.clone();
                let render_context = context.clone();
                prepared = spawn_blocking(move || {
                    prepared.render_png(&output_for_worker, &render_context)?;
                    Ok::<_, MotionError>(prepared)
                })
                .await
                .map_err(|error| MotionError::render(format!("PNG worker failed: {error}")))??;
                let _ = prepared;
            }
            MotionFormat::Mp4 => {
                let ffmpeg = required_ffmpeg(self.ffmpeg_path.as_deref())?;
                prepared = render_mp4(prepared, &ffmpeg, &output, &context).await?;
            }
            MotionFormat::Gif => {
                let ffmpeg = required_ffmpeg(self.ffmpeg_path.as_deref())?;
                let palette = context
                    .output_path("palette.png")
                    .map_err(|error| MotionError::render(error.to_string()))?;
                prepared = render_gif(prepared, &ffmpeg, &palette, &output, &context).await?;
            }
        }
        let _ = prepared;
        let encode_elapsed = encode_started.elapsed();
        let output_bytes = tokio::fs::metadata(&output)
            .await
            .map_err(|error| MotionError::render(format!("output metadata failed: {error}")))?
            .len();
        eprintln!(
            "Motion generation completed: format={} frames={frame_count} dimensions={}x{} asset_ms={} prepare_ms={} encode_ms={} total_ms={} output_bytes={output_bytes}",
            extension,
            dimensions.0,
            dimensions.1,
            asset_elapsed.as_millis(),
            prepare_elapsed.as_millis(),
            encode_elapsed.as_millis(),
            total_started.elapsed().as_millis(),
        );
        Ok(TaskArtifact::new(
            output,
            format!("{}.{}", self.plan.filename_stem, extension),
            Some(
                match self.plan.format {
                    MotionFormat::Png => "image/png",
                    MotionFormat::Mp4 => "video/mp4",
                    MotionFormat::Gif => "image/gif",
                }
                .to_owned(),
            ),
            None,
        ))
    }
}

struct PreparedMotion {
    project: MotionProject,
    frames: Vec<FrameRef>,
    rasterizer: Rasterizer,
    previous_packets: Option<Vec<DrawPacket>>,
}

impl PreparedMotion {
    fn new(
        plan: MotionPlan,
        assets: super::assets::MotionAssets,
        context: &TaskContext,
    ) -> Result<Self, MotionError> {
        context.report("⏳ モーションデータを解析しています");
        let project = MotionProject::parse(&assets)?;
        let sprite = SpriteSheet::decode(&assets.sprite, &project.cuts)?;
        let frames = resolve_frames(&plan, &project)?;
        let total = frames.len();
        let layout = measure(&plan, &project, &sprite, &frames, |completed| {
            if context.cancellation().is_cancelled() {
                return Err(MotionError::render("motion measurement was cancelled"));
            }
            context.report(format!("⏳ 表示範囲を計測しています ({completed}/{total})"));
            Ok(())
        })?;
        let rasterizer = Rasterizer::new(sprite, layout)?;
        Ok(Self {
            project,
            frames,
            rasterizer,
            previous_packets: None,
        })
    }

    fn render_png(&mut self, output: &Path, context: &TaskContext) -> Result<(), MotionError> {
        context.report("⏳ PNGを描画しています");
        let frame = self.frames[0];
        let packets = evaluate(&self.project, frame.motion, frame.frame)?;
        let (width, height) = self.rasterizer.dimensions();
        let rgba = self.rasterizer.render(&packets)?;
        let file = File::create(output)
            .map_err(|error| MotionError::render(format!("PNG create failed: {error}")))?;
        let mut encoder = Encoder::new(BufWriter::new(file), width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_compression(Compression::Fast);
        encoder.set_filter(Filter::Adaptive);
        let mut writer = encoder
            .write_header()
            .map_err(|error| MotionError::render(format!("PNG header failed: {error}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| MotionError::render(format!("PNG encode failed: {error}")))
    }

    fn render_rgba_frame(&mut self, index: usize) -> Result<bool, MotionError> {
        let frame = self.frames[index];
        let packets = evaluate(&self.project, frame.motion, frame.frame)?;
        let reused = self
            .previous_packets
            .as_ref()
            .is_some_and(|previous| previous == &packets);
        if !reused {
            self.rasterizer.render(&packets)?;
            self.previous_packets = Some(packets);
        }
        Ok(reused)
    }

    fn rgba(&self) -> &[u8] {
        self.rasterizer.rgba()
    }

    fn dimensions(&self) -> (u32, u32) {
        self.rasterizer.dimensions()
    }
}

async fn render_mp4(
    mut prepared: PreparedMotion,
    ffmpeg: &Path,
    output: &Path,
    context: &TaskContext,
) -> Result<PreparedMotion, MotionError> {
    let (width, height) = prepared.dimensions();
    let arguments = vec![
        "-y".to_owned(),
        "-hide_banner".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-threads".to_owned(),
        "1".to_owned(),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "rgba".to_owned(),
        "-video_size".to_owned(),
        format!("{width}x{height}"),
        "-framerate".to_owned(),
        FRAME_RATE.to_string(),
        "-i".to_owned(),
        "pipe:0".to_owned(),
        "-c:v".to_owned(),
        "libx264".to_owned(),
        "-preset".to_owned(),
        "ultrafast".to_owned(),
        "-crf".to_owned(),
        "23".to_owned(),
        "-pix_fmt".to_owned(),
        "yuv420p".to_owned(),
        "-movflags".to_owned(),
        "+faststart".to_owned(),
        "-an".to_owned(),
        output.to_string_lossy().into_owned(),
    ];
    let (mut child, mut stdin, stderr) = start_encoder(ffmpeg, &arguments)?;
    let total = prepared.frames.len();
    for index in 0..total {
        if context.cancellation().is_cancelled() {
            stop_encoder(&mut child).await;
            return Err(MotionError::render("MP4 rendering was cancelled"));
        }
        prepared = match render_frame(prepared, index, context).await {
            Ok(prepared) => prepared,
            Err(error) => {
                stop_encoder(&mut child).await;
                return Err(error);
            }
        };
        context.report(format!("⏳ MP4を描画しています ({}/{total})", index + 1));
        if let Err(error) = write_rgba(&mut stdin, prepared.rgba(), context).await {
            stop_encoder(&mut child).await;
            return Err(error);
        }
    }
    drop(stdin);
    finish_encoder(child, stderr).await?;
    Ok(prepared)
}

async fn render_gif(
    mut prepared: PreparedMotion,
    ffmpeg: &Path,
    palette: &Path,
    output: &Path,
    context: &TaskContext,
) -> Result<PreparedMotion, MotionError> {
    let (width, height) = prepared.dimensions();
    let palette_arguments = vec![
        "-y".to_owned(),
        "-hide_banner".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-threads".to_owned(),
        "1".to_owned(),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "rgba".to_owned(),
        "-video_size".to_owned(),
        format!("{width}x{height}"),
        "-framerate".to_owned(),
        "30".to_owned(),
        "-i".to_owned(),
        "pipe:0".to_owned(),
        "-vf".to_owned(),
        "palettegen=reserve_transparent=1".to_owned(),
        "-frames:v".to_owned(),
        "1".to_owned(),
        palette.to_string_lossy().into_owned(),
    ];
    let (palette_child, mut palette_stdin, palette_stderr) =
        start_encoder(ffmpeg, &palette_arguments)?;
    for index in sample_indices(prepared.frames.len()) {
        prepared = render_frame(prepared, index, context).await?;
        write_rgba(&mut palette_stdin, prepared.rgba(), context).await?;
    }
    drop(palette_stdin);
    finish_encoder(palette_child, palette_stderr).await?;

    prepared.previous_packets = None;
    let arguments = vec![
        "-y".to_owned(),
        "-hide_banner".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-threads".to_owned(),
        "1".to_owned(),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "rgba".to_owned(),
        "-video_size".to_owned(),
        format!("{width}x{height}"),
        "-framerate".to_owned(),
        FRAME_RATE.to_string(),
        "-i".to_owned(),
        "pipe:0".to_owned(),
        "-i".to_owned(),
        palette.to_string_lossy().into_owned(),
        "-filter_complex".to_owned(),
        "[0:v][1:v]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle".to_owned(),
        "-loop".to_owned(),
        "0".to_owned(),
        output.to_string_lossy().into_owned(),
    ];
    let (mut child, mut stdin, stderr) = start_encoder(ffmpeg, &arguments)?;
    let total = prepared.frames.len();
    for index in 0..total {
        prepared = match render_frame(prepared, index, context).await {
            Ok(prepared) => prepared,
            Err(error) => {
                stop_encoder(&mut child).await;
                return Err(error);
            }
        };
        context.report(format!("⏳ GIFを描画しています ({}/{total})", index + 1));
        if let Err(error) = write_rgba(&mut stdin, prepared.rgba(), context).await {
            stop_encoder(&mut child).await;
            return Err(error);
        }
    }
    drop(stdin);
    finish_encoder(child, stderr).await?;
    Ok(prepared)
}

async fn render_frame(
    mut prepared: PreparedMotion,
    index: usize,
    context: &TaskContext,
) -> Result<PreparedMotion, MotionError> {
    if context.cancellation().is_cancelled() {
        return Err(MotionError::render("motion rendering was cancelled"));
    }
    prepared = spawn_blocking(move || {
        prepared.render_rgba_frame(index)?;
        Ok::<_, MotionError>(prepared)
    })
    .await
    .map_err(|error| MotionError::render(format!("frame worker failed: {error}")))??;
    Ok(prepared)
}

async fn write_rgba(
    stdin: &mut ChildStdin,
    rgba: &[u8],
    context: &TaskContext,
) -> Result<(), MotionError> {
    tokio::select! {
        biased;
        _ = context.cancellation().cancelled() => Err(MotionError::render("encoder write was cancelled")),
        result = stdin.write_all(rgba) => result
            .map_err(|error| MotionError::render(format!("encoder write failed: {error}"))),
    }
}

fn start_encoder(
    ffmpeg: &Path,
    arguments: &[String],
) -> Result<(Child, ChildStdin, tokio::process::ChildStderr), MotionError> {
    let mut child = Command::new(ffmpeg)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| MotionError::render(format!("FFmpeg start failed: {error}")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| MotionError::render("FFmpeg stdin is unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| MotionError::render("FFmpeg stderr is unavailable"))?;
    Ok((child, stdin, stderr))
}

async fn finish_encoder(
    mut child: Child,
    mut stderr: tokio::process::ChildStderr,
) -> Result<(), MotionError> {
    let stderr_task = tokio::spawn(async move {
        let mut data = Vec::new();
        let _ = stderr.read_to_end(&mut data).await;
        if data.len() > STDERR_LIMIT {
            data.drain(..data.len() - STDERR_LIMIT);
        }
        String::from_utf8_lossy(&data).into_owned()
    });
    let status = child
        .wait()
        .await
        .map_err(|error| MotionError::render(format!("FFmpeg wait failed: {error}")))?;
    let stderr = stderr_task.await.unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(MotionError::render(format!(
            "FFmpeg exited with {status}: {stderr}"
        )))
    }
}

async fn stop_encoder(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn sample_indices(frame_count: usize) -> Vec<usize> {
    let count = frame_count.min(PALETTE_SAMPLE_COUNT);
    if count <= 1 {
        return vec![0];
    }
    (0..count)
        .map(|index| index * (frame_count - 1) / (count - 1))
        .collect()
}

fn required_ffmpeg(path: Option<&Path>) -> Result<PathBuf, MotionError> {
    path.filter(|path| path.is_file())
        .map(Path::to_owned)
        .ok_or_else(|| MotionError::render("FFmpeg executable is not configured"))
}
