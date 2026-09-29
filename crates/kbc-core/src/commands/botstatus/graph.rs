//! CPU・Memoryの時系列をDiscord添付用PNGへ描画する。

use std::fmt::{Display, Formatter};

use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use super::model::MetricSeries;

const WIDTH: u32 = 1_000;
const HEIGHT: u32 = 480;
const LEFT: f32 = 36.0;
const RIGHT: f32 = 20.0;
const TOP: f32 = 24.0;
const PANEL_HEIGHT: f32 = 196.0;
const PANEL_GAP: f32 = 38.0;

pub(super) fn render_graph(
    cpu: &MetricSeries,
    memory: &MetricSeries,
) -> Result<Vec<u8>, GraphError> {
    let mut pixmap = Pixmap::new(WIDTH, HEIGHT).ok_or(GraphError::Allocation)?;
    pixmap.fill(Color::from_rgba8(0x18, 0x1b, 0x22, 0xff));
    draw_panel(
        &mut pixmap,
        cpu,
        TOP,
        Color::from_rgba8(0x58, 0xa6, 0xff, 0xff),
    );
    draw_panel(
        &mut pixmap,
        memory,
        TOP + PANEL_HEIGHT + PANEL_GAP,
        Color::from_rgba8(0xbc, 0x8c, 0xff, 0xff),
    );

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| GraphError::Encoding(error.to_string()))?;
        writer
            .write_image_data(pixmap.data())
            .map_err(|error| GraphError::Encoding(error.to_string()))?;
    }
    Ok(bytes)
}

fn draw_panel(pixmap: &mut Pixmap, series: &MetricSeries, top: f32, color: Color) {
    let width = WIDTH as f32 - LEFT - RIGHT;
    let Some(rect) = Rect::from_xywh(LEFT, top, width, PANEL_HEIGHT) else {
        return;
    };
    let mut background = Paint::default();
    background.set_color_rgba8(0x21, 0x26, 0x30, 0xff);
    pixmap.fill_rect(rect, &background, Transform::identity(), None);

    let mut grid = Paint::default();
    grid.set_color_rgba8(0x3a, 0x42, 0x50, 0xff);
    let mut grid_stroke = Stroke::default();
    grid_stroke.width = 1.0;
    for index in 0..=4 {
        let y = top + PANEL_HEIGHT * index as f32 / 4.0;
        draw_line(pixmap, LEFT, y, LEFT + width, y, &grid, &grid_stroke);
    }
    for index in 0..=6 {
        let x = LEFT + width * index as f32 / 6.0;
        draw_line(pixmap, x, top, x, top + PANEL_HEIGHT, &grid, &grid_stroke);
    }

    if series.points.is_empty() {
        return;
    }
    let maximum = graph_maximum(series);
    let first_time = series
        .points
        .first()
        .expect("checked")
        .at
        .timestamp_millis();
    let last_time = series.points.last().expect("checked").at.timestamp_millis();
    let time_span = (last_time - first_time).max(1) as f64;
    let mut path = PathBuilder::new();
    for (index, point) in series.points.iter().enumerate() {
        let x =
            LEFT + width * ((point.at.timestamp_millis() - first_time) as f64 / time_span) as f32;
        let normalized = (point.value.max(0.0) / maximum).clamp(0.0, 1.0) as f32;
        let y = top + PANEL_HEIGHT * (1.0 - normalized);
        if index == 0 {
            path.move_to(x, y);
        } else {
            path.line_to(x, y);
        }
    }
    let Some(path) = path.finish() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let mut stroke = Stroke::default();
    stroke.width = 3.0;
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn draw_line(
    pixmap: &mut Pixmap,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    paint: &Paint<'_>,
    stroke: &Stroke,
) {
    let mut path = PathBuilder::new();
    path.move_to(x1, y1);
    path.line_to(x2, y2);
    if let Some(path) = path.finish() {
        pixmap.stroke_path(&path, paint, stroke, Transform::identity(), None);
    }
}

fn graph_maximum(series: &MetricSeries) -> f64 {
    if series.unit == "pct" {
        return 100.0;
    }
    series.maximum().unwrap_or(1.0).max(1.0) * 1.1
}

#[derive(Debug)]
pub(super) enum GraphError {
    Allocation,
    Encoding(String),
}

impl Display for GraphError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allocation => formatter.write_str("status graph allocation failed"),
            Self::Encoding(reason) => write!(formatter, "status graph encoding failed: {reason}"),
        }
    }
}
