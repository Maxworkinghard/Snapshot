//! 拖动桌宠时播的走路动作，把"横穿画布"的改成"原地走"。
//!
//! 有的素材包把走路画成人物从宽画布的一头走到另一头（740×410 里横走将近 300px）。
//! 桌宠窗口是方的，整张缩进去人物只剩二十几像素高，还在窗口里滑来滑去；拖动时窗口本身
//! 已经跟着鼠标在走，要的是原地踏步。这里每帧跟着人物裁一个固定宽度的框，高度和地面线
//! 对齐默认动作：拖起来和待机时一样大，脚踩在同一条线上。本来就原地走的不动。

use super::*;
use image::{codecs::gif::GifDecoder, AnimationDecoder, Rgba};

/// 人物中心的横向移动超过自身宽度的这个比例，才算横穿画布
const TRAVEL_RATIO: f64 = 0.5;

/// 背景画进 GIF 里的素材，和背景色差多少才算人物
const SOLID_TOLERANCE: i16 = 24;

/// 哪些像素算人物：带透明色的看 alpha；整张不透明的（背景画进去了）看和背景色差多少
#[derive(Clone, Copy)]
pub(crate) enum Backdrop {
    Transparent,
    Solid([u8; 3]),
}

impl Backdrop {
    pub(crate) fn of(frames: &[RgbaImage]) -> Self {
        if frames
            .iter()
            .any(|frame| frame.pixels().any(|pixel| pixel[3] < 255))
        {
            return Backdrop::Transparent;
        }
        // 四周一圈出现最多的颜色当背景
        let first = &frames[0];
        let (width, height) = first.dimensions();
        let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
        for (x, y, pixel) in first.enumerate_pixels() {
            if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
                *counts.entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
            }
        }
        let color = counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(color, _)| color)
            .unwrap_or([255, 255, 255]);
        Backdrop::Solid(color)
    }

    fn is_figure(self, pixel: &Rgba<u8>) -> bool {
        match self {
            Backdrop::Transparent => pixel[3] > 0,
            Backdrop::Solid(color) => (0..3)
                .any(|index| (pixel[index] as i16 - color[index] as i16).abs() > SOLID_TOLERANCE),
        }
    }

    fn fill(self) -> Rgba<u8> {
        match self {
            Backdrop::Transparent => Rgba([0, 0, 0, 0]),
            Backdrop::Solid([r, g, b]) => Rgba([r, g, b, 255]),
        }
    }
}

/// 人物外框 [左, 上, 右, 下)，这一帧里没有人物就是 None
fn figure_box(frame: &RgbaImage, backdrop: Backdrop) -> Option<[u32; 4]> {
    let mut bounds: Option<[u32; 4]> = None;
    for (x, y, pixel) in frame.enumerate_pixels() {
        if backdrop.is_figure(pixel) {
            let current = bounds.get_or_insert([x, y, x + 1, y + 1]);
            current[0] = current[0].min(x);
            current[1] = current[1].min(y);
            current[2] = current[2].max(x + 1);
            current[3] = current[3].max(y + 1);
        }
    }
    bounds
}

/// 解出每一帧（已按处置方式合成成整张画布）和它的时长（百分之一秒）
pub(crate) fn decode(bytes: &[u8]) -> Result<Vec<(RgbaImage, u16)>, String> {
    let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let frames = decoder
        .into_frames()
        .collect_frames()
        .map_err(|error| error.to_string())?;
    Ok(frames
        .into_iter()
        .map(|frame| {
            let (numer, denom) = frame.delay().numer_denom_ms();
            let centis = (numer as f64 / denom.max(1) as f64 / 10.0).round() as u16;
            (frame.into_buffer(), centis)
        })
        .collect())
}

pub(crate) fn encode(
    frames: Vec<(RgbaImage, u16)>,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let (width, height) = (width as u16, height as u16);
    let mut bytes = Vec::new();
    {
        let mut encoder =
            gif::Encoder::new(&mut bytes, width, height, &[]).map_err(|error| error.to_string())?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|error| error.to_string())?;
        for (image, delay) in frames {
            let mut pixels = image.into_raw();
            let mut frame = gif::Frame::from_rgba_speed(width, height, &mut pixels, 10);
            frame.delay = delay;
            // 每帧画完清成透明再画下一帧，不然手脚摆过的地方会留下残影
            frame.dispose = gif::DisposalMethod::Background;
            encoder
                .write_frame(&frame)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(bytes)
}

/// 最小二乘拟合一条直线，返回 (斜率, 截距)
fn fit_line(points: &[(f64, f64)]) -> (f64, f64) {
    let count = points.len() as f64;
    let mean_x = points.iter().map(|point| point.0).sum::<f64>() / count;
    let mean_y = points.iter().map(|point| point.1).sum::<f64>() / count;
    let spread = points
        .iter()
        .map(|point| (point.0 - mean_x).powi(2))
        .sum::<f64>();
    let slope = if spread > 0.0 {
        points
            .iter()
            .map(|point| (point.0 - mean_x) * (point.1 - mean_y))
            .sum::<f64>()
            / spread
    } else {
        0.0
    };
    (slope, mean_y - slope * mean_x)
}

/// 横穿画布的走路动画改成原地走；本来就原地走的返回 None，照用原图。
/// `idle` 是这个形象的默认动作，输出的高度和地面线跟它对齐。
pub(crate) fn walk_in_place(walk: &[u8], idle: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let frames = decode(walk)?;
    if frames.len() < 2 {
        return Ok(None);
    }
    let images: Vec<RgbaImage> = frames.iter().map(|(image, _)| image.clone()).collect();
    let backdrop = Backdrop::of(&images);
    let boxes: Vec<(usize, [u32; 4])> = images
        .iter()
        .enumerate()
        .filter_map(|(index, image)| figure_box(image, backdrop).map(|bounds| (index, bounds)))
        .collect();
    if boxes.len() < 2 {
        return Ok(None);
    }
    let mut widths: Vec<u32> = boxes
        .iter()
        .map(|(_, bounds)| bounds[2] - bounds[0])
        .collect();
    widths.sort_unstable();
    let centers: Vec<(f64, f64)> = boxes
        .iter()
        .map(|(index, bounds)| (*index as f64, (bounds[0] + bounds[2]) as f64 / 2.0))
        .collect();
    let lowest = centers.iter().map(|point| point.1).fold(f64::MAX, f64::min);
    let highest = centers.iter().map(|point| point.1).fold(f64::MIN, f64::max);
    if highest - lowest <= widths[widths.len() / 2] as f64 * TRAVEL_RATIO {
        return Ok(None);
    }

    // 默认动作画在方形窗口里时，缩放由长边决定，脚底在方框里的位置由它的外框决定
    let idle_frame = image::load_from_memory_with_format(idle, ImageFormat::Gif)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let idle_backdrop = Backdrop::of(std::slice::from_ref(&idle_frame));
    let (idle_width, idle_height) = idle_frame.dimensions();
    let side = idle_width.max(idle_height);
    let idle_bottom = figure_box(&idle_frame, idle_backdrop)
        .map(|bounds| bounds[3])
        .unwrap_or(idle_height);
    let ground = (side - idle_height) / 2 + idle_bottom;

    // 输出是竖长的（宽不超过长边），这样在方形窗口里的缩放和默认动作一样
    let widest = *widths.last().unwrap_or(&side);
    let width = (widest + side / 10).clamp(1, side);
    let height = side;
    let walk_bottom = boxes
        .iter()
        .map(|(_, bounds)| bounds[3])
        .max()
        .unwrap_or(height);
    let shift_y = ground as i64 - walk_bottom as i64;
    // 跟着人物中心裁框：手脚摆动会让外框忽宽忽窄，直接用外框中心会左右抖，按帧号拟合成匀速
    let (slope, intercept) = fit_line(&centers);
    let along_line = |index: usize| slope * index as f64 + intercept;
    // 走走停停的（nashor 钻地：先原地钻下去，再横着钻过去，最后原地钻出来）直线跟不上，
    // 顺着直线裁会把人物裁掉一块，这时改成每帧对准人物中心。比框还宽的帧怎么裁都会缺，不算
    let follow_line = boxes.iter().all(|&(index, bounds)| {
        let left = (along_line(index) - width as f64 / 2.0).round();
        bounds[2] - bounds[0] > width
            || (bounds[0] as f64 >= left && bounds[2] as f64 <= left + width as f64)
    });
    let mut figure_centers = vec![None; images.len()];
    for &(index, bounds) in &boxes {
        figure_centers[index] = Some((bounds[0] + bounds[2]) as f64 / 2.0);
    }

    let mut output = Vec::with_capacity(frames.len());
    for (index, (image, delay)) in frames.into_iter().enumerate() {
        let center = match figure_centers[index] {
            Some(center) if !follow_line => center,
            _ => along_line(index),
        };
        let left = (center - width as f64 / 2.0).round() as i64;
        let mut canvas = RgbaImage::from_pixel(width, height, backdrop.fill());
        for y in 0..height {
            let source_y = y as i64 - shift_y;
            if source_y < 0 || source_y >= image.height() as i64 {
                continue;
            }
            for x in 0..width {
                let source_x = left + x as i64;
                if source_x < 0 || source_x >= image.width() as i64 {
                    continue;
                }
                canvas.put_pixel(x, y, *image.get_pixel(source_x as u32, source_y as u32));
            }
        }
        output.push((canvas, delay));
    }
    encode(output, width, height).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Rgba<u8> = Rgba([30, 30, 40, 255]);
    const CLEAR: Rgba<u8> = Rgba([0, 0, 0, 0]);
    const PAPER: Rgba<u8> = Rgba([254, 254, 254, 255]);

    /// 每帧在 `canvas` 上画一个 `size` 大小的色块，左上角依次在 `spots`
    fn block_gif(
        canvas: (u32, u32),
        background: Rgba<u8>,
        size: (u32, u32),
        spots: &[(u32, u32)],
    ) -> Vec<u8> {
        let frames = spots
            .iter()
            .map(|&(left, top)| {
                let mut image = RgbaImage::from_pixel(canvas.0, canvas.1, background);
                for y in top..top + size.1 {
                    for x in left..left + size.0 {
                        image.put_pixel(x, y, INK);
                    }
                }
                (image, 10)
            })
            .collect();
        encode(frames, canvas.0, canvas.1).expect("生成测试 GIF 失败")
    }

    /// 横走：色块在 160 宽的画布上从左走到右，而且画得比待机高（脚没踩在底边）
    fn walk_across(background: Rgba<u8>) -> Vec<u8> {
        let spots: Vec<(u32, u32)> = (0..8).map(|index| (10 + index * 18, 10)).collect();
        block_gif((160, 80), background, (10, 30), &spots)
    }

    #[test]
    fn a_walk_across_the_canvas_becomes_a_walk_in_place() {
        // 待机：40×80 的画布，脚踩在底边
        let idle = block_gif((40, 80), CLEAR, (10, 30), &[(15, 50), (15, 50)]);
        let walked = walk_in_place(&walk_across(CLEAR), &idle)
            .expect("处理失败")
            .expect("横穿画布的应当被改成原地走");
        let frames = decode(&walked).expect("输出解不开");
        assert_eq!(frames.len(), 8);
        for (image, delay) in &frames {
            // 高度跟待机一样（长边 80），宽度是色块加余量，竖长
            assert_eq!(image.dimensions(), (18, 80));
            assert_eq!(*delay, 10);
            let bounds = figure_box(image, Backdrop::Transparent).expect("人物不见了");
            let center = (bounds[0] + bounds[2]) as f64 / 2.0;
            assert!(
                (center - 9.0).abs() <= 1.0,
                "人物应当在框中间，实际中心 {center}"
            );
            assert_eq!(bounds[3], 80, "脚底应当和待机一样踩在底边");
        }
    }

    #[test]
    fn a_stop_and_go_walk_is_kept_centered() {
        // 走走停停：先原地待着，再一下横过去，最后停住（nashor 钻地就是这样），直线跟不上
        let idle = block_gif((40, 80), CLEAR, (10, 30), &[(15, 50), (15, 50)]);
        let spots = [
            (10, 10),
            (10, 10),
            (10, 10),
            (60, 10),
            (110, 10),
            (140, 10),
            (140, 10),
            (140, 10),
        ];
        let walked = walk_in_place(&block_gif((160, 80), CLEAR, (10, 30), &spots), &idle)
            .expect("处理失败")
            .expect("横穿画布的应当被改成原地走");
        for (image, _) in &decode(&walked).expect("输出解不开") {
            let bounds = figure_box(image, Backdrop::Transparent).expect("人物不见了");
            assert_eq!(bounds[2] - bounds[0], 10, "人物被裁掉了一块");
            let center = (bounds[0] + bounds[2]) as f64 / 2.0;
            assert!(
                (center - 9.0).abs() <= 1.0,
                "人物应当在框中间，实际中心 {center}"
            );
        }
    }

    #[test]
    fn a_walk_in_place_is_left_alone() {
        let idle = block_gif((40, 80), CLEAR, (10, 30), &[(15, 50), (15, 50)]);
        let marching = block_gif(
            (40, 80),
            CLEAR,
            (10, 30),
            &[(15, 50), (16, 50), (15, 50), (14, 50)],
        );
        assert!(walk_in_place(&marching, &idle).expect("处理失败").is_none());
    }

    #[test]
    fn a_background_painted_into_the_gif_is_kept_as_padding() {
        let idle = block_gif((40, 80), PAPER, (10, 30), &[(15, 50), (15, 50)]);
        let walked = walk_in_place(&walk_across(PAPER), &idle)
            .expect("处理失败")
            .expect("横穿画布的应当被改成原地走");
        let frames = decode(&walked).expect("输出解不开");
        for (image, _) in &frames {
            // 补出来的地方用原来的背景色，不会变成透明，和包里其它动作一致
            assert_eq!(*image.get_pixel(0, 0), PAPER);
            let bounds = figure_box(image, Backdrop::Solid([254, 254, 254])).expect("人物不见了");
            assert_eq!(bounds[3], 80);
        }
    }

    #[test]
    fn something_that_is_not_a_gif_is_an_error_not_a_panic() {
        assert!(walk_in_place(b"not a gif", b"nor this").is_err());
    }
}
