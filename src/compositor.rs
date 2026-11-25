//! Frame compositor (§8 z-order, §12 background crop + layer overdraw).
//!
//! Per-frame pipeline:
//!   1. Fill `FrameBuffer` opaque black.
//!   2. Blit background crop `[CropWindow]` to frame at (0,0), clipped.
//!   3. Draw `ResolvedPose` parts z-ascending (painter's algorithm, §8).
//!
//! **M1 scope:**
//!   - No dirty-rect — full-frame redraw (§12 defers to M3).
//!   - No rotation — parts axis-aligned at `top_left` (M2 adds it).
//!   - Background assumed pre-loaded at scale 1.0 (§12: "background bada
//!     image, camera crop window khisaye"). Character `height` background
//!     ko affect nahi karta.
//!
//! **Frame opacity contract:** output frame is always alpha=255 (video frame
//! has no alpha channel in MP4). Background semi-transparent pixels are
//! forced to 255; black padding shows through the rest.

use crate::assets::CachedImage;
use crate::camera::CropWindow;
use crate::skeleton::ResolvedPose;

pub struct FrameBuffer {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl FrameBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0u8; (width as usize) * (height as usize) * 4],
        }
    }

    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

/// In-place: `frame` ko background crop + parts se bharo.
pub fn compose_frame(
    frame: &mut FrameBuffer,
    background: &CachedImage,
    crop: &CropWindow,
    pose: &ResolvedPose,
) {
    fill_opaque_black(frame);
    blit_background_crop(frame, background, crop);
    for part in &pose.parts {
        blit_part(frame, part.top_left, &part.image);
    }
}

fn fill_opaque_black(frame: &mut FrameBuffer) {
    for px in frame.rgba.chunks_exact_mut(4) {
        px[0] = 0;
        px[1] = 0;
        px[2] = 0;
        px[3] = 255;
    }
}

fn blit_background_crop(frame: &mut FrameBuffer, bg: &CachedImage, crop: &CropWindow) {
    let copy_w = crop
        .w
        .min(frame.width)
        .min(bg.width.saturating_sub(crop.x));
    let copy_h = crop
        .h
        .min(frame.height)
        .min(bg.height.saturating_sub(crop.y));
    if copy_w == 0 || copy_h == 0 {
        return;
    }

    for dy in 0..copy_h {
        let sy = crop.y + dy;
        let src_start = ((sy as usize) * (bg.width as usize) + (crop.x as usize)) * 4;
        let src_end = src_start + (copy_w as usize) * 4;
        let dst_start = ((dy as usize) * (frame.width as usize)) * 4;
        let dst_end = dst_start + (copy_w as usize) * 4;

        let src_row = &bg.rgba[src_start..src_end];
        let dst_row = &mut frame.rgba[dst_start..dst_end];
        for (s, d) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
            d[0] = s[0];
            d[1] = s[1];
            d[2] = s[2];
            d[3] = 255; // frame stays opaque
        }
    }
}

fn blit_part(frame: &mut FrameBuffer, top_left: (f32, f32), img: &CachedImage) {
    // M1: no rotation → round to nearest pixel.
    let px = top_left.0.round() as i32;
    let py = top_left.1.round() as i32;
    let iw = img.width as i32;
    let ih = img.height as i32;
    let fw = frame.width as i32;
    let fh = frame.height as i32;

    let sx0 = (-px).max(0);
    let sy0 = (-py).max(0);
    let sx1 = (fw - px).min(iw);
    let sy1 = (fh - py).min(ih);
    if sx0 >= sx1 || sy0 >= sy1 {
        return; // fully off-screen
    }

    for sy in sy0..sy1 {
        let dy = py + sy;
        if dy < 0 || dy >= fh {
            continue;
        }
        let src_row_start = (sy as usize) * (img.width as usize) * 4;
        let dst_row_start = (dy as usize) * (frame.width as usize) * 4;
        for sx in sx0..sx1 {
            let dx = px + sx;
            if dx < 0 || dx >= fw {
                continue;
            }
            let si = src_row_start + (sx as usize) * 4;
            let di = dst_row_start + (dx as usize) * 4;
            blend_pixel(&img.rgba[si..si + 4], &mut frame.rgba[di..di + 4]);
        }
    }
}

/// Standard source-over alpha blend. `dst` is assumed opaque (invariant of
/// this pipeline: bg drawn first, alpha forced to 255). `sa == 0` skips,
/// `sa == 255` fast copy.
#[inline]
fn blend_pixel(src: &[u8], dst: &mut [u8]) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    if sa == 255 {
        dst[0] = src[0];
        dst[1] = src[1];
        dst[2] = src[2];
        return;
    }
    let inv = 255 - sa;
    dst[0] = ((src[0] as u32 * sa + dst[0] as u32 * inv + 127) / 255) as u8;
    dst[1] = ((src[1] as u32 * sa + dst[1] as u32 * inv + 127) / 255) as u8;
    dst[2] = ((src[2] as u32 * sa + dst[2] as u32 * inv + 127) / 255) as u8;
    // dst[3] stays 255 — frame invariant.
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::ResolvedPart;
    use std::sync::Arc;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> CachedImage {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            pixels.extend_from_slice(&rgba);
        }
        CachedImage { rgba: pixels, width: w, height: h, src_width: w, src_height: h }
    }

    fn from_fn(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> CachedImage {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&f(x, y));
            }
        }
        CachedImage { rgba: pixels, width: w, height: h, src_width: w, src_height: h }
    }

    fn part(name: &str, z: i32, top_left: (f32, f32), img: CachedImage) -> ResolvedPart {
        ResolvedPart {
            name: name.to_string(),
            z,
            image: Arc::new(img),
            top_left,
            world_pivot: (0.0, 0.0),
        }
    }

    fn pose_from(mut parts: Vec<ResolvedPart>) -> ResolvedPose {
        parts.sort_by(|a, b| a.z.cmp(&b.z).then_with(|| a.name.cmp(&b.name)));
        ResolvedPose { parts }
    }

    fn px(frame: &FrameBuffer, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * frame.width + x) * 4) as usize;
        [frame.rgba[i], frame.rgba[i + 1], frame.rgba[i + 2], frame.rgba[i + 3]]
    }

    fn full_crop(w: u32, h: u32) -> CropWindow {
        CropWindow { x: 0, y: 0, w, h }
    }

    // ---------------- background only ----------------

    #[test]
    fn solid_background_fills_frame() {
        let mut frame = FrameBuffer::new(10, 10);
        let bg = solid(10, 10, [255, 0, 0, 255]);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose_from(vec![]));
        for y in 0..10 {
            for x in 0..10 {
                assert_eq!(px(&frame, x, y), [255, 0, 0, 255], "at ({x},{y})");
            }
        }
    }

    #[test]
    fn crop_window_offsets_background() {
        // bg(x,y) = (x*10, y*10, 0, 255); crop starts at (5, 10).
        let bg = from_fn(20, 20, |x, y| [(x as u8) * 10, (y as u8) * 10, 0, 255]);
        let mut frame = FrameBuffer::new(10, 8);
        let crop = CropWindow { x: 5, y: 10, w: 10, h: 8 };
        compose_frame(&mut frame, &bg, &crop, &pose_from(vec![]));
        assert_eq!(px(&frame, 0, 0), [50, 100, 0, 255]);
        assert_eq!(px(&frame, 9, 7), [140, 170, 0, 255]);
    }

    #[test]
    fn background_smaller_than_frame_padded_black() {
        let bg = solid(5, 5, [255, 0, 0, 255]);
        let mut frame = FrameBuffer::new(10, 10);
        let crop = CropWindow { x: 0, y: 0, w: 5, h: 5 };
        compose_frame(&mut frame, &bg, &crop, &pose_from(vec![]));
        assert_eq!(px(&frame, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&frame, 4, 4), [255, 0, 0, 255]);
        assert_eq!(px(&frame, 5, 0), [0, 0, 0, 255]);
        assert_eq!(px(&frame, 9, 9), [0, 0, 0, 255]);
    }

    #[test]
    fn background_semi_transparent_forced_opaque() {
        // Frame invariant: alpha=255 always.
        let bg = solid(4, 4, [255, 0, 0, 128]);
        let mut frame = FrameBuffer::new(4, 4);
        compose_frame(&mut frame, &bg, &full_crop(4, 4), &pose_from(vec![]));
        assert_eq!(px(&frame, 0, 0)[3], 255);
        assert_eq!(px(&frame, 3, 3)[3], 255);
    }

    // ---------------- parts ----------------

    #[test]
    fn part_opaque_draws_over_background() {
        let bg = solid(10, 10, [255, 0, 0, 255]);
        let p = solid(2, 2, [0, 0, 255, 255]);
        let pose = pose_from(vec![part("p1", 10, (3.0, 4.0), p)]);
        let mut frame = FrameBuffer::new(10, 10);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);

        assert_eq!(px(&frame, 3, 4), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 4, 5), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 2, 4), [255, 0, 0, 255], "left of part untouched");
        assert_eq!(px(&frame, 5, 4), [255, 0, 0, 255], "right of part untouched");
    }

    #[test]
    fn semi_transparent_part_blends_with_background() {
        // src green a=128 over red bg → [127, 128, 0, 255] (±1 for rounding).
        let bg = solid(4, 4, [255, 0, 0, 255]);
        let p = solid(1, 1, [0, 255, 0, 128]);
        let pose = pose_from(vec![part("p1", 1, (0.0, 0.0), p)]);
        let mut frame = FrameBuffer::new(4, 4);
        compose_frame(&mut frame, &bg, &full_crop(4, 4), &pose);

        let got = px(&frame, 0, 0);
        assert!((got[0] as i32 - 127).abs() <= 1, "r={}", got[0]);
        assert!((got[1] as i32 - 128).abs() <= 1, "g={}", got[1]);
        assert_eq!(got[2], 0);
        assert_eq!(got[3], 255);
    }

    #[test]
    fn fully_transparent_part_is_noop() {
        let bg = solid(4, 4, [255, 0, 0, 255]);
        let p = solid(2, 2, [0, 0, 255, 0]);
        let pose = pose_from(vec![part("p1", 1, (0.0, 0.0), p)]);
        let mut frame = FrameBuffer::new(4, 4);
        compose_frame(&mut frame, &bg, &full_crop(4, 4), &pose);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(px(&frame, x, y), [255, 0, 0, 255], "at ({x},{y})");
            }
        }
    }

    #[test]
    fn z_order_higher_z_drawn_on_top() {
        let bg = solid(4, 4, [0, 0, 0, 255]);
        let red = solid(2, 2, [255, 0, 0, 255]);
        let blue = solid(2, 2, [0, 0, 255, 255]);
        // pose_from sorts by z; blue z=2 > red z=1 → blue drawn later.
        let pose = pose_from(vec![
            part("red", 1, (0.0, 0.0), red),
            part("blue", 2, (0.0, 0.0), blue),
        ]);
        let mut frame = FrameBuffer::new(4, 4);
        compose_frame(&mut frame, &bg, &full_crop(4, 4), &pose);
        assert_eq!(px(&frame, 0, 0), [0, 0, 255, 255], "blue must win at overlap");
    }

    // ---------------- clipping ----------------

    #[test]
    fn part_offscreen_left_clipped() {
        // Part 3x3 at (-2, 3): only src column 2 (dst_x=0) visible.
        let bg = solid(10, 10, [255, 0, 0, 255]);
        let p = solid(3, 3, [0, 0, 255, 255]);
        let pose = pose_from(vec![part("p1", 1, (-2.0, 3.0), p)]);
        let mut frame = FrameBuffer::new(10, 10);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);

        for dy in 3..6 {
            assert_eq!(px(&frame, 0, dy), [0, 0, 255, 255], "column 0 at y={dy}");
            assert_eq!(px(&frame, 1, dy), [255, 0, 0, 255], "column 1 should be bg");
        }
        assert_eq!(px(&frame, 0, 2), [255, 0, 0, 255], "above part");
        assert_eq!(px(&frame, 0, 6), [255, 0, 0, 255], "below part");
    }

    #[test]
    fn part_offscreen_right_clipped() {
        // Part 5x5 at (8, 0) in 10-wide frame → only columns 0,1 (src) visible.
        let bg = solid(10, 10, [255, 0, 0, 255]);
        let p = solid(5, 5, [0, 0, 255, 255]);
        let pose = pose_from(vec![part("p1", 1, (8.0, 0.0), p)]);
        let mut frame = FrameBuffer::new(10, 10);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);

        assert_eq!(px(&frame, 8, 0), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 9, 4), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 7, 0), [255, 0, 0, 255], "left edge is bg");
    }

    #[test]
    fn fully_offscreen_part_no_crash() {
        let bg = solid(4, 4, [255, 0, 0, 255]);
        let p = solid(3, 3, [0, 0, 255, 255]);
        let pose = pose_from(vec![part("p1", 1, (-100.0, -100.0), p)]);
        let mut frame = FrameBuffer::new(4, 4);
        compose_frame(&mut frame, &bg, &full_crop(4, 4), &pose);
        // Everything still red.
        assert_eq!(px(&frame, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&frame, 3, 3), [255, 0, 0, 255]);
    }

    // ---------------- integration-ish ----------------

    #[test]
    fn crop_plus_part_combined() {
        // bg solid red, crop offset, part sits inside crop coordinate space.
        let bg = solid(20, 20, [255, 0, 0, 255]);
        let p = solid(2, 2, [0, 255, 0, 255]);
        let pose = pose_from(vec![part("p1", 1, (2.0, 2.0), p)]);
        let mut frame = FrameBuffer::new(10, 10);
        let crop = CropWindow { x: 5, y: 5, w: 10, h: 10 };
        compose_frame(&mut frame, &bg, &crop, &pose);

        assert_eq!(px(&frame, 0, 0), [255, 0, 0, 255], "crop corner is bg");
        assert_eq!(px(&frame, 2, 2), [0, 255, 0, 255], "part at frame coord (2,2)");
    }

    #[test]
    fn fractional_top_left_rounds() {
        // top_left (1.4, 2.6) → rounds to (1, 3).
        let bg = solid(10, 10, [255, 0, 0, 255]);
        let p = solid(1, 1, [0, 0, 255, 255]);
        let pose = pose_from(vec![part("p1", 1, (1.4, 2.6), p)]);
        let mut frame = FrameBuffer::new(10, 10);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);
        assert_eq!(px(&frame, 1, 3), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 1, 2), [255, 0, 0, 255]);
    }
      }
