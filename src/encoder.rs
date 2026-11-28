//! Encoder (§10, M4): raw RGBA frames -> FFmpeg pipe -> ek piece ki MP4.
//!
//! Sirf `std` use hota hai (naya crate nahi). Ek `PieceEncoder` = ek piece
//! (5 second) ki MP4. Frame banao, `write_frame` se pipe mein bhejo, aur
//! frame turant hat jata hai (§11: frames memory mein jama nahi hote).
//!
//! FFmpeg settings (preset, crf, path) config.rs se aate hain, yahan hard-code
//! nahi. H.264 (yuv420p) ke liye width aur height dono even hone chahiye.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::JoinHandle;

use crate::config::Config;

/// FFmpeg ke error text ka max hissa jo error message mein jayega.
const MAX_ERR_CHARS: usize = 500;

/// Ek piece ki MP4 banane wala encoder.
pub struct PieceEncoder {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stderr_thread: Option<JoinHandle<String>>,
    width: u32,
    height: u32,
    frames_written: u32,
}

/// Frame size valid hai ya nahi (H.264 ke liye even, aur non-zero).
fn validate_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err(format!("video size must be non-zero, got {}x{}", width, height));
    }
    if width % 2 != 0 || height % 2 != 0 {
        return Err(format!(
            "width and height must be even for H.264, got {}x{}",
            width, height
        ));
    }
    Ok(())
}

/// Ek RGBA frame ki lambai w*h*4 honi chahiye.
fn check_frame_len(width: u32, height: u32, len: usize) -> Result<(), String> {
    let expected = width as usize * height as usize * 4;
    if len != expected {
        return Err(format!(
            "frame has {} bytes, expected {} ({}x{} RGBA)",
            len, expected, width, height
        ));
    }
    Ok(())
}

/// FFmpeg ke command-line arguments. Alag function taaki FFmpeg ke bina bhi test ho.
fn build_args(cfg: &Config, width: u32, height: u32, out: &Path) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pix_fmt".to_string(),
        "rgba".to_string(),
        "-s".to_string(),
        format!("{}x{}", width, height),
        "-r".to_string(),
        cfg.fps.to_string(),
        "-i".to_string(),
        "pipe:0".to_string(),
        "-an".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        cfg.ffmpeg_preset.to_string(),
        "-crf".to_string(),
        cfg.ffmpeg_crf.to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        out.to_string_lossy().to_string(),
    ]
}

fn truncate(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= MAX_ERR_CHARS {
        t.to_string()
    } else {
        t.chars().take(MAX_ERR_CHARS).collect()
    }
}

impl PieceEncoder {
    /// FFmpeg chalu karta hai. `out` pe piece ki MP4 banegi.
    pub fn start(cfg: &Config, width: u32, height: u32, out: &Path) -> Result<Self, String> {
        validate_size(width, height)?;

        let mut child = Command::new(cfg.ffmpeg_path)
            .args(build_args(cfg, width, height, out))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg start failed ({}): {}", cfg.ffmpeg_path, e))?;

        let stdin = child.stdin.take();
        let mut stderr = child.stderr.take();

        // stderr alag thread mein padhte hain, warna pipe bhar jaye to FFmpeg ruk sakta hai.
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(s) = stderr.as_mut() {
                let _ = s.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).to_string()
        });

        Ok(Self {
            child: Some(child),
            stdin,
            stderr_thread: Some(stderr_thread),
            width,
            height,
            frames_written: 0,
        })
    }

    /// Ek RGBA frame pipe mein bhejta hai. Size galat ho to error.
    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<(), String> {
        check_frame_len(self.width, self.height, rgba.len())?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "encoder already finished".to_string())?;
        stdin
            .write_all(rgba)
            .map_err(|e| format!("ffmpeg pipe write failed: {}", e))?;
        self.frames_written += 1;
        Ok(())
    }

    /// Pipe band karke FFmpeg ka exit status dekhta hai. Ok mein frames ki ginti.
    pub fn finish(mut self) -> Result<u32, String> {
        // stdin band hone par hi FFmpeg ko pata chalta hai ki frames khatam.
        self.stdin.take();

        let mut child = self
            .child
            .take()
            .ok_or_else(|| "encoder already finished".to_string())?;
        let status = child
            .wait()
            .map_err(|e| format!("ffmpeg wait failed: {}", e))?;

        let err_text = match self.stderr_thread.take() {
            Some(h) => h.join().unwrap_or_default(),
            None => String::new(),
        };

        if !status.success() {
            return Err(format!("ffmpeg exited with {}: {}", status, truncate(&err_text)));
        }
        Ok(self.frames_written)
    }
}

impl Drop for PieceEncoder {
    /// finish() ke bina drop hua (jaise error pe) to FFmpeg ko band kar do.
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Ek piece mein kitne frames (fps * piece_sec, kam se kam 1).
pub fn frames_per_piece(fps: u32, piece_sec: f32) -> u32 {
    ((fps as f32 * piece_sec).round() as u32).max(1)
}

/// Frame number kis piece (0 se shuru) mein aata hai.
pub fn piece_for_frame(frame: u32, fps: u32, piece_sec: f32) -> u32 {
    frame / frames_per_piece(fps, piece_sec)
}

/// Piece ki file ka naam, jaise piece_000.mp4.
pub fn piece_file_name(piece: u32) -> String {
    format!("piece_{:03}.mp4", piece)
}

/// FFmpeg chal sakta hai ya nahi (`ffmpeg -version`).
pub fn ffmpeg_available(path: &str) -> bool {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_odd_size() {
        let cfg = Config::default();
        let err = PieceEncoder::start(&cfg, 201, 200, Path::new("x.mp4"))
            .err()
            .expect("odd width must fail");
        assert!(err.contains("even"), "unexpected error: {}", err);
        assert!(PieceEncoder::start(&cfg, 200, 201, Path::new("x.mp4")).is_err());
    }

    #[test]
    fn rejects_zero_size() {
        let cfg = Config::default();
        assert!(PieceEncoder::start(&cfg, 0, 200, Path::new("x.mp4")).is_err());
        assert!(PieceEncoder::start(&cfg, 200, 0, Path::new("x.mp4")).is_err());
    }

    #[test]
    fn rejects_wrong_frame_len() {
        assert!(check_frame_len(200, 200, 200 * 200 * 4).is_ok());
        assert!(check_frame_len(200, 200, 200 * 200 * 4 - 1).is_err());
        assert!(check_frame_len(200, 200, 0).is_err());
    }

    #[test]
    fn piece_index_math() {
        // 15 fps, 5 sec = 75 frames per piece.
        assert_eq!(frames_per_piece(15, 5.0), 75);
        assert_eq!(piece_for_frame(0, 15, 5.0), 0);
        assert_eq!(piece_for_frame(74, 15, 5.0), 0);
        assert_eq!(piece_for_frame(75, 15, 5.0), 1);
        assert_eq!(piece_for_frame(299, 15, 5.0), 3);
        // bahut chhota piece bhi kam se kam 1 frame ka.
        assert_eq!(frames_per_piece(15, 0.001), 1);
    }

    #[test]
    fn piece_file_names() {
        assert_eq!(piece_file_name(0), "piece_000.mp4");
        assert_eq!(piece_file_name(12), "piece_012.mp4");
    }

    #[test]
    fn ffmpeg_args_follow_config() {
        let cfg = Config::default();
        let args = build_args(&cfg, 200, 200, Path::new("out.mp4"));
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"ultrafast".to_string()));
        assert!(args.contains(&"200x200".to_string()));
        assert!(args.contains(&"pipe:0".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn missing_ffmpeg_binary_is_unavailable() {
        assert!(!ffmpeg_available("definitely-not-a-real-ffmpeg-binary-xyz"));
    }

    #[test]
    fn writes_mp4() {
        let cfg = Config::default();
        if !ffmpeg_available(cfg.ffmpeg_path) {
            eprintln!("ffmpeg not found, skipping writes_mp4");
            return;
        }
        let out = std::env::temp_dir().join(format!("lkj_encoder_test_{}.mp4", std::process::id()));
        let (w, h) = (64u32, 64u32);
        let mut enc = PieceEncoder::start(&cfg, w, h, &out).expect("start");
        for i in 0..15u32 {
            let shade = (i * 16) as u8;
            let mut frame = vec![0u8; (w * h * 4) as usize];
            for px in frame.chunks_mut(4) {
                px[0] = shade;
                px[1] = 80;
                px[2] = 200;
                px[3] = 255;
            }
            enc.write_frame(&frame).expect("write_frame");
        }
        let n = enc.finish().expect("finish");
        assert_eq!(n, 15);

        let bytes = std::fs::read(&out).expect("mp4 file must exist");
        let _ = std::fs::remove_file(&out);
        assert!(bytes.len() > 8, "mp4 too small: {} bytes", bytes.len());
        assert_eq!(&bytes[4..8], b"ftyp", "not an MP4 (no ftyp box)");
    }
}
