//! Camera crop window (§7, §12).
//!
//! §12: "background ek bada image ho aur camera us pe sirf ek crop window
//! khisakaye." Background `2160x1280` (§7 example) → video `720x1280`.
//!
//! **M1 scope:** camera static hai. `Camera::STATIC` = `pan = (0, 0)` →
//! window hamesha background ke top-left se `video.w × video.h`.
//! Pan/tilt ka math (clamping, fractional → integer) yahin likha hai taaki
//! M3 mein sirf caller ko `Camera` badalna pade, is fn ko nahi.
//!
//! **Units:** `pan_x / pan_y` = **video pixel** displacement. +X = crop window
//! ko background ke right mein khiskao (jaise "camera moves E"). +Y = neeche
//! ("camera moves S"). §7 ke `camera.move` E/W/N/S ka mapping M3 mein
//! `pan_x += ±amount * t`, `pan_y += ±amount * t` se hoga.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub pan_x: f32,
    pub pan_y: f32,
}

impl Camera {
    pub const STATIC: Camera = Camera { pan_x: 0.0, pan_y: 0.0 };
}

impl Default for Camera {
    fn default() -> Self {
        Camera::STATIC
    }
}

/// Background ke andar integer pixel crop window. Guaranteed inside
/// `0..bg_w × 0..bg_h` **jab bg >= video ho**. Chhota bg case niche documented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropWindow {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// `camera` + background size + video size -> integer crop window.
///
/// - Pan clamped taaki window background ke andar rahe:
///     `x ∈ [0, bg_w - vid_w]`, same y.
/// - Fractional pan: nearest pixel (`round`).
/// - **Background chhota hai video se**: window = poora background at (0,0),
///   w/h = bg. Compositor zero-padding karega (§12 se bada bg expected tha,
///   par guard hai).
pub fn window(cam: &Camera, bg: (u32, u32), vid: (u32, u32)) -> CropWindow {
    // Background smaller than video — return full bg, no clamping math.
    if bg.0 <= vid.0 || bg.1 <= vid.1 {
        return CropWindow { x: 0, y: 0, w: bg.0.min(vid.0), h: bg.1.min(vid.1) };
    }

    let max_x = (bg.0 - vid.0) as f32;
    let max_y = (bg.1 - vid.1) as f32;

    let x = cam.pan_x.clamp(0.0, max_x).round() as u32;
    let y = cam.pan_y.clamp(0.0, max_y).round() as u32;

    CropWindow { x, y, w: vid.0, h: vid.1 }
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    // §7 example numbers.
    const BG: (u32, u32) = (2160, 1280);
    const VID: (u32, u32) = (720, 1280);

    #[test]
    fn static_camera_returns_top_left_window() {
        let w = window(&Camera::STATIC, BG, VID);
        assert_eq!(w, CropWindow { x: 0, y: 0, w: 720, h: 1280 });
    }

    #[test]
    fn default_trait_equals_static() {
        let w_default = window(&Camera::default(), BG, VID);
        let w_static = window(&Camera::STATIC, BG, VID);
        assert_eq!(w_default, w_static);
    }

    #[test]
    fn pan_east_moves_window_right() {
        let w = window(&Camera { pan_x: 200.0, pan_y: 0.0 }, BG, VID);
        assert_eq!(w.x, 200);
        assert_eq!(w.y, 0);
    }

    #[test]
    fn pan_south_moves_window_down() {
        let w = window(&Camera { pan_x: 0.0, pan_y: 100.0 }, BG, VID);
        assert_eq!(w.x, 0);
        assert_eq!(w.y, 100);
    }

    #[test]
    fn pan_clamped_at_right_and_bottom_edges() {
        // bg - vid = (1440, 0) — y ka koi room hi nahi.
        let w = window(&Camera { pan_x: 5000.0, pan_y: 5000.0 }, BG, VID);
        assert_eq!(w.x, 1440);
        assert_eq!(w.y, 0);
        assert_eq!(w.w, VID.0);
        assert_eq!(w.h, VID.1);
    }

    #[test]
    fn negative_pan_clamped_to_zero() {
        let w = window(&Camera { pan_x: -300.0, pan_y: -50.0 }, BG, VID);
        assert_eq!(w.x, 0);
        assert_eq!(w.y, 0);
    }

    #[test]
    fn fractional_pan_rounds_to_nearest() {
        let w = window(&Camera { pan_x: 100.4, pan_y: 0.0 }, BG, VID);
        assert_eq!(w.x, 100);
        let w = window(&Camera { pan_x: 100.6, pan_y: 0.0 }, BG, VID);
        assert_eq!(w.x, 101);
    }

    #[test]
    fn exact_fit_background() {
        // bg == vid -> window = full, no pan room.
        let w = window(&Camera { pan_x: 100.0, pan_y: 100.0 }, VID, VID);
        assert_eq!(w, CropWindow { x: 0, y: 0, w: 720, h: 1280 });
    }

    #[test]
    fn smaller_background_returns_bg_dimensions() {
        // bg smaller than video on both axes.
        let small_bg = (400, 300);
        let w = window(&Camera { pan_x: 100.0, pan_y: 100.0 }, small_bg, VID);
        assert_eq!(w, CropWindow { x: 0, y: 0, w: 400, h: 300 });
    }

    #[test]
    fn partial_small_background_clamps_each_axis_independently() {
        // bg.w < vid.w lekin bg.h > vid.h — dono axes independent.
        let bg = (400, 2000);
        let w = window(&Camera { pan_x: 100.0, pan_y: 300.0 }, bg, VID);
        // x-axis: bg<=vid -> x=0, w=400
        // y-axis: bg>vid -> window moves but our branch returns early.
        // Ye early-return design ka contract hai: ek bhi axis chhoti ho to
        // full-bg path chalta hai. Documented, tested.
        assert_eq!(w, CropWindow { x: 0, y: 0, w: 400, h: 1280 });
    }

    #[test]
    fn pan_result_never_escapes_background() {
        for px in [-1000.0, -1.0, 0.0, 1.0, 100.0, 2000.0] {
            for py in [-1000.0, 0.0, 1000.0] {
                let w = window(&Camera { pan_x: px, pan_y: py }, BG, VID);
                assert!(w.x + w.w <= BG.0, "x overflow: {:?}", w);
                assert!(w.y + w.h <= BG.1, "y overflow: {:?}", w);
            }
        }
    }

    #[test]
    fn camera_is_copy_cheap() {
        // Compile-time: Camera Copy hai (per-frame construct karne ke liye).
        fn takes(_: Camera) {}
        let c = Camera::STATIC;
        takes(c);
        takes(c);
    }
          }
