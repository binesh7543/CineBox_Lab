//! Tunable defaults (§15) + hard limits (§3, §11).
//!
//! Ye file hi single source of truth hai tunable values ke liye. Config::default()
//! §15 ke numbers deta hai. Per-job overrides (jaise video.w) main.rs/api.rs se
//! aayenge — yahan hard-code nahi karna.
//!
//! Hard-limits (HARD_MAX_*) ko kabhi nahi badhana. Config::validate() startup pe
//! panic karta hai agar koi unhe cross kare.
//!
//! M4/M5.0.1: FFmpeg settings, delivery retry count aur environment variable
//! ke naam bhi yahin hain. Secrets (token, key) kabhi yahan likhe nahi jate,
//! sirf unke env-var ke naam yahan hain; value Render ke environment se aati hai.

// ---- Hard limits (document §3, §11) — never raise ----
pub const HARD_MAX_CACHE_BYTES: usize = 150 * 1024 * 1024;
pub const HARD_MAX_FRAME_BUFFERS: usize = 3;

// ---- Environment variable names (values Render dashboard se aayengi) ----
/// Shared secret, request header X-Engine-Key se match hota hai (§6).
pub const ENGINE_KEY_ENV: &str = "ENGINE_KEY";
/// Telegram bot token. Repo mein kabhi nahi.
pub const TELEGRAM_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
/// Telegram group ka chat id (group ke liye negative number hota hai).
pub const TELEGRAM_CHAT_ID_ENV: &str = "TELEGRAM_CHAT_ID";
/// Render free service ko ek HTTP port chahiye, wo is variable mein aata hai.
pub const PORT_ENV: &str = "PORT";
/// Port jab PORT env set na ho (local test).
pub const DEFAULT_PORT: u16 = 8080;

/// Tunable engine defaults. Section 15.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub video_width: u32,
    pub video_height: u32,
    pub fps: u32,
    pub piece_sec: f32,
    pub default_duration_sec: f32,
    pub skin_base_height: u32,
    pub asset_cache_bytes: usize,
    pub frame_buffer_count: usize,
    /// FFmpeg binary ka naam ya path (Dockerfile mein PATH pe rakhenge).
    pub ffmpeg_path: &'static str,
    /// FFmpeg preset (§12: ultrafast).
    pub ffmpeg_preset: &'static str,
    /// H.264 quality, 0 (best) se 51 (worst). Test se tay hoga.
    pub ffmpeg_crf: u32,
    /// Piece bhejne ki koshishein (§13: 3 baar retry).
    pub delivery_retries: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            video_width: 720,
            video_height: 1280,
            fps: 15,
            piece_sec: 5.0,
            default_duration_sec: 20.0,
            skin_base_height: 400,
            asset_cache_bytes: 150 * 1024 * 1024,
            frame_buffer_count: 3,
            ffmpeg_path: "ffmpeg",
            ffmpeg_preset: "ultrafast",
            ffmpeg_crf: 28,
            delivery_retries: 3,
        }
    }
}

impl Config {
    /// Engine startup pe ek baar call hota hai. Hard limits violate ho to panic.
    pub fn validate(&self) {
        assert!(
            self.asset_cache_bytes <= HARD_MAX_CACHE_BYTES,
            "asset_cache_bytes {} exceeds hard limit {}",
            self.asset_cache_bytes,
            HARD_MAX_CACHE_BYTES,
        );
        assert!(
            self.frame_buffer_count > 0 && self.frame_buffer_count <= HARD_MAX_FRAME_BUFFERS,
            "frame_buffer_count {} out of range (1..={})",
            self.frame_buffer_count,
            HARD_MAX_FRAME_BUFFERS,
        );
        assert!(self.video_width > 0 && self.video_height > 0, "video size must be non-zero");
        assert!(self.fps > 0, "fps must be non-zero");
        assert!(self.piece_sec > 0.0, "piece_sec must be > 0");
        assert!(self.default_duration_sec > 0.0, "default_duration_sec must be > 0");
        assert!(self.skin_base_height > 0, "skin_base_height must be > 0");
        assert!(!self.ffmpeg_path.is_empty(), "ffmpeg_path must not be empty");
        assert!(!self.ffmpeg_preset.is_empty(), "ffmpeg_preset must not be empty");
        assert!(self.ffmpeg_crf <= 51, "ffmpeg_crf must be 0..=51");
        assert!(self.delivery_retries > 0, "delivery_retries must be > 0");
    }

    /// Ek RGBA frame buffer ke bytes.
    pub fn frame_bytes(&self) -> usize {
        self.video_width as usize * self.video_height as usize * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_section_15() {
        let c = Config::default();
        assert_eq!(c.video_width, 720);
        assert_eq!(c.video_height, 1280);
        assert_eq!(c.fps, 15);
        assert_eq!(c.piece_sec, 5.0);
        assert_eq!(c.default_duration_sec, 20.0);
        assert_eq!(c.skin_base_height, 400);
        assert_eq!(c.asset_cache_bytes, 150 * 1024 * 1024);
        assert_eq!(c.frame_buffer_count, 3);
    }

    #[test]
    fn ffmpeg_and_delivery_defaults() {
        let c = Config::default();
        assert_eq!(c.ffmpeg_path, "ffmpeg");
        assert_eq!(c.ffmpeg_preset, "ultrafast");
        assert_eq!(c.ffmpeg_crf, 28);
        assert_eq!(c.delivery_retries, 3);
    }

    #[test]
    fn defaults_pass_validate() {
        Config::default().validate();
    }

    #[test]
    #[should_panic(expected = "exceeds hard limit")]
    fn cache_above_hard_limit_panics() {
        let mut c = Config::default();
        c.asset_cache_bytes = HARD_MAX_CACHE_BYTES + 1;
        c.validate();
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn zero_frame_buffers_panics() {
        let mut c = Config::default();
        c.frame_buffer_count = 0;
        c.validate();
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn too_many_frame_buffers_panics() {
        let mut c = Config::default();
        c.frame_buffer_count = 4;
        c.validate();
    }

    #[test]
    #[should_panic(expected = "video size must be non-zero")]
    fn zero_video_size_panics() {
        let mut c = Config::default();
        c.video_width = 0;
        c.validate();
    }

    #[test]
    #[should_panic(expected = "ffmpeg_crf must be 0..=51")]
    fn crf_above_51_panics() {
        let mut c = Config::default();
        c.ffmpeg_crf = 52;
        c.validate();
    }

    #[test]
    fn frame_bytes_is_w_h_4() {
        let c = Config::default();
        // 720 * 1280 * 4 = 3,686,400
        assert_eq!(c.frame_bytes(), 3_686_400);
    }

    #[test]
    fn frame_buffers_fit_ram_budget() {
        // §11: frame buffers ko ~25 MB ke andar rehna hai.
        let c = Config::default();
        let total = c.frame_bytes() * c.frame_buffer_count;
        assert!(
            total <= 25 * 1024 * 1024,
            "frame buffers {} bytes exceed 25 MB §11 budget",
            total,
        );
    }
}
