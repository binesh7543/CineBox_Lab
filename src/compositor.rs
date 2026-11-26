//! Frame compositor (§8 z-order, §12 background crop + layer overdraw).
//!
//! M2 Update:
//!   - Rotation support (Nearest Neighbor sampling).
//!   - AABB (Bounding Box) optimization: only rotated bounding box is processed.
//!   - `blit_part` takes `ResolvedPart` directly to access angle and world_pivot.
//!
//! M3 Camera Fix:
//!   - Background ke saath-saath characters (parts) bhi `CropWindow` ke hisaab se shift honge,
//!     taaki camera pan/tilt karne par poori screen ek saath move kare.

use crate::assets::CachedImage;
use crate::camera::CropWindow;
use crate::skeleton::{ResolvedPose, ResolvedPart};

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
        blit_part(frame, part, crop);
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
    let copy_w = crop.w.min(frame.width).min(bg.width.saturating_sub(crop.x));
    let copy_h = crop.h.min(frame.height).min(bg.height.saturating_sub(crop.y));
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
            d[3] = 255;
        }
    }
}

fn blit_part(frame: &mut FrameBuffer, part: &ResolvedPart, crop: &CropWindow) {
    // Camera offset nikaalo (taaki character background ke saath move kare)
    let offset_x = crop.x as f32;
    let offset_y = crop.y as f32;

    let screen_top_left = (part.top_left.0 - offset_x, part.top_left.1 - offset_y);
    let screen_world_pivot = (part.world_pivot.0 - offset_x, part.world_pivot.1 - offset_y);

    // Fast path: if angle is effectively zero, use direct memory copy (M1 logic).
    if part.angle_rad.abs() < 1e-5 {
        blit_part_unrotated(frame, screen_top_left, &part.image);
    } else {
        blit_part_rotated(frame, screen_top_left, screen_world_pivot, part.angle_rad, &part.image);
    }
}

fn blit_part_unrotated(frame: &mut FrameBuffer, top_left: (f32, f32), img: &CachedImage) {
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
        return;
    }

    for sy in sy0..sy1 {
        let dy = py + sy;
        if dy < 0 || dy >= fh { continue; }

        let src_row_start = (sy as usize) * (img.width as usize) * 4;
        let dst_row_start = (dy as usize) * (frame.width as usize) * 4;
        for sx in sx0..sx1 {
            let dx = px + sx;
            if dx < 0 || dx >= fw { continue; }

            let si = src_row_start + (sx as usize) * 4;
            let di = dst_row_start + (dx as usize) * 4;
            blend_pixel(&img.rgba[si..si + 4], &mut frame.rgba[di..di + 4]);
        }
    }
}

fn blit_part_rotated(
    frame: &mut FrameBuffer,
    top_left: (f32, f32),
    world_pivot: (f32, f32),
    angle_rad: f32,
    img: &CachedImage,
) {
    let w = img.width as f32;
    let h = img.height as f32;
    let (px, py) = world_pivot;
    let (tx, ty) = top_left;
    let angle = angle_rad;

    let cos_a = angle.cos();
    let sin_a = angle.sin();

    // Calculate AABB (Bounding Box) of the rotated image
    let corners = [(tx, ty), (tx + w, ty), (tx, ty + h), (tx + w, ty + h)];
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for &(cx, cy) in &corners {
        let dx = cx - px;
        let dy = cy - py;
        let rx = dx * cos_a - dy * sin_a + px;
        let ry = dx * sin_a + dy * cos_a + py;
        min_x = min_x.min(rx);
        max_x = max_x.max(rx);
        min_y = min_y.min(ry);
        max_y = max_y.max(ry);
    }

    // Clip AABB to frame bounds
    let start_x = (min_x.floor() as i32).max(0);
    let end_x = (max_x.ceil() as i32).min(frame.width as i32);
    let start_y = (min_y.floor() as i32).max(0);
    let end_y = (max_y.ceil() as i32).min(frame.height as i32);

    if start_x >= end_x || start_y >= end_y {
        return;
    }

    // Inverse rotation mapping (Nearest Neighbor)
    for dy in start_y..end_y {
        for dx in start_x..end_x {
            // Destination pixel center
            let wx = dx as f32 + 0.5;
            let wy = dy as f32 + 0.5;

            // Translate to pivot center
            let cx = wx - px;
            let cy = wy - py;

            // Inverse rotate
            let rx = cx * cos_a + cy * sin_a;
            let ry = -cx * sin_a + cy * cos_a;

            // Translate back to image space
            let sx = rx + px - tx;
            let sy = ry + py - ty;

            // Check if inside source image
            if sx >= 0.0 && sx < w && sy >= 0.0 && sy < h {
                let px_x = sx as usize;
                let px_y = sy as usize;

                let src_idx = (px_y * img.width as usize + px_x) * 4;
                let dst_idx = ((dy as usize) * frame.width as usize + dx as usize) * 4;

                blend_pixel(
                    &img.rgba[src_idx..src_idx + 4],
                    &mut frame.rgba[dst_idx..dst_idx + 4],
                );
            }
        }
    }
}

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
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> CachedImage {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            pixels.extend_from_slice(&rgba);
        }
        CachedImage { rgba: pixels, width: w, height: h, src_width: w, src_height: h }
    }

    fn part(name: &str, z: i32, top_left: (f32, f32), img: CachedImage, angle: f32) -> ResolvedPart {
        // Pivot is assumed to be at the center of the image for these tests
        let px = top_left.0 + (img.width as f32 / 2.0);
        let py = top_left.1 + (img.height as f32 / 2.0);
        ResolvedPart {
            name: name.to_string(),
            z,
            image: Arc::new(img),
            top_left,
            world_pivot: (px, py),
            angle_rad: angle,
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

    #[test]
    fn part_unrotated_fast_path() {
        let bg = solid(10, 10, [255, 0, 0, 255]);
        let p = solid(2, 2, [0, 0, 255, 255]);
        let pose = pose_from(vec![part("p1", 10, (3.0, 4.0), p, 0.0)]);
        let mut frame = FrameBuffer::new(10, 10);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);

        assert_eq!(px(&frame, 3, 4), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 4, 5), [0, 0, 255, 255]);
    }

    #[test]
    fn rotated_part_90deg_hits_correct_pixels() {
        let mut img = solid(2, 1, [0, 0, 0, 0]); // Transparent base
        img.rgba[0..4].copy_from_slice(&[255, 0, 0, 255]); // Red
        img.rgba[4..8].copy_from_slice(&[0, 0, 255, 255]); // Blue

        let pose = pose_from(vec![part("p1", 1, (2.0, 4.0), img, std::f32::consts::FRAC_PI_2)]);
        let mut frame = FrameBuffer::new(10, 10);
        let bg = solid(10, 10, [0, 0, 0, 0]); 
        
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);

        // After 90 deg rotation, Red goes to top-left area of the new box, Blue goes to bottom-right.
        assert_eq!(px(&frame, 3, 3), [255, 0, 0, 255]);
        assert_eq!(px(&frame, 3, 4), [0, 0, 255, 255]);
    }

    #[test]
    fn rotated_part_45deg_expands_aabb() {
        let bg = solid(10, 10, [255, 0, 0, 255]);
        let p = solid(2, 2, [0, 0, 255, 255]); // 2x2 square
        let pose = pose_from(vec![part("p1", 1, (4.0, 4.0), p, std::f32::consts::FRAC_PI_4)]);
        let mut frame = FrameBuffer::new(10, 10);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);

        // Center is (5,5). 45 deg rotation moves the corners out.
        // Pixel (5,4) and (4,5) should be covered by the blue square.
        assert_eq!(px(&frame, 5, 4), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 4, 5), [0, 0, 255, 255]);
    }

    #[test]
    fn rotated_off_screen_no_crash() {
        let bg = solid(10, 10, [255, 0, 0, 255]);
        let p = solid(2, 2, [0, 0, 255, 255]);
        let pose = pose_from(vec![part("p1", 1, (-50.0, -50.0), p, 1.0)]);
        let mut frame = FrameBuffer::new(10, 10);
        compose_frame(&mut frame, &bg, &full_crop(10, 10), &pose);
        // Should not crash, should just not draw anything
        assert_eq!(px(&frame, 0, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn camera_pan_shifts_parts_with_background() {
        let bg = solid(20, 20, [255, 0, 0, 255]);
        let p = solid(2, 2, [0, 0, 255, 255]);
        
        // Part is at world position (10, 10)
        let pose = pose_from(vec![part("p1", 1, (10.0, 10.0), p, 0.0)]);
        let mut frame = FrameBuffer::new(10, 10);
        
        // Camera pan x=5, y=0
        let crop = CropWindow { x: 5, y: 0, w: 10, h: 10 };
        compose_frame(&mut frame, &bg, &crop, &pose);

        // Original part at 10,10 should now be at 10 - 5 = 5, 10 on screen
        assert_eq!(px(&frame, 5, 10), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 4, 10), [255, 0, 0, 255]); // Outside part
    }
}
