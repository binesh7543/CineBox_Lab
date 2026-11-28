//! Engine entry point: CLI (PNG frames ya MP4 pieces) aur server mode.
//!
//! M3: camera pan/tilt aur face (aankh/mooh) timeline se resolve hote hain.
//! M4: --video flag: frames PNG ke bajaye FFmpeg ko jate hain, har piece_sec
//!     (5 sec) ka ek piece_NNN.mp4 banta hai.
//! M5.0.1: --serve flag: HTTP API (api.rs). Render ka kaam yahi se hota hai
//!     (render_job_pieces), piece ready hote hi callback se Telegram delivery.
//!
//! Usage:
//!   lkj-engine <job.json> <out_dir> [assets_root]            -> PNG frames (pehle jaisa)
//!   lkj-engine <job.json> <out_dir> [assets_root] --video    -> piece_NNN.mp4 files
//!   lkj-engine --serve [assets_root] [work_dir]              -> HTTP server

mod api;
mod assets;
mod camera;
mod compositor;
mod config;
mod delivery;
mod encoder;
mod face;
mod json_stream;
mod skeleton;
mod status;
mod timeline;
mod types;

use assets::{AssetLoader, CachedImage};
use camera::Camera;
use compositor::{compose_frame, FrameBuffer};
use config::Config;
use json_stream::{parse_job, parse_skin};
use skeleton::{resolve_animated_pose, ResolvedPart, ResolvedPose};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use types::{Character, Job, SkinManifest};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let raw: Vec<String> = std::env::args().collect();
    let prog = raw.first().cloned().unwrap_or_else(|| "lkj-engine".to_string());
    let video_mode = raw.iter().any(|a| a == "--video");
    let serve_mode = raw.iter().any(|a| a == "--serve");
    // Flags hata ke sirf positional arguments.
    let args: Vec<String> = raw
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();

    let cfg = Config::default();
    cfg.validate();

    if serve_mode {
        let assets_root = PathBuf::from(args.first().cloned().unwrap_or_else(|| ".".to_string()));
        let work_dir = PathBuf::from(
            args.get(1)
                .cloned()
                .unwrap_or_else(|| "/tmp/lkj_jobs".to_string()),
        );
        return api::serve(cfg, assets_root, work_dir);
    }

    if args.len() < 2 {
        eprintln!("usage: {prog} <job.json> <out_dir> [assets_root] [--video]");
        eprintln!("       {prog} --serve [assets_root] [work_dir]");
        std::process::exit(2);
    }
    let job_path = PathBuf::from(&args[0]);
    let out_dir = PathBuf::from(&args[1]);
    let assets_root = args.get(2).cloned().unwrap_or_else(|| ".".to_string());

    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("create out_dir {}: {e}", out_dir.display()))?;

    let job = load_job(&job_path)?;

    if video_mode {
        if !encoder::ffmpeg_available(cfg.ffmpeg_path) {
            return Err(format!("ffmpeg not found (looked for '{}')", cfg.ffmpeg_path));
        }
        let n = render_job_pieces(
            &job,
            &cfg,
            Path::new(&assets_root),
            &out_dir,
            &mut |piece, path| {
                println!("piece {} ready: {}", piece, path.display());
                Ok(())
            },
        )?;
        println!("{} piece(s) written to {}", n, out_dir.display());
    } else {
        render_job_frames(&job, &cfg, Path::new(&assets_root), &out_dir)?;
    }
    Ok(())
}

fn load_job(path: &Path) -> Result<Job, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open job {}: {e}", path.display()))?;
    parse_job(file).map_err(|e| format!("job json {}: {e}", path.display()))
}

fn dummy_image() -> Arc<CachedImage> {
    Arc::new(CachedImage {
        rgba: vec![0, 0, 0, 0],
        width: 1,
        height: 1,
        src_width: 1,
        src_height: 1,
    })
}

/// Total frames: duration * fps (upar ki taraf).
fn total_frames(job: &Job, cfg: &Config) -> u32 {
    let fps = job.video.fps.unwrap_or(cfg.fps);
    let duration = job.video.duration_sec.unwrap_or(1.0);
    (duration * fps as f32).ceil() as u32
}

/// Is job ke kitne pieces banenge (api.rs status ke liye pehle se batata hai).
pub(crate) fn count_pieces(job: &Job, cfg: &Config) -> u32 {
    let fps = job.video.fps.unwrap_or(cfg.fps);
    let piece_sec = job.video.piece_sec.unwrap_or(cfg.piece_sec);
    let per = encoder::frames_per_piece(fps, piece_sec);
    let total = total_frames(job, cfg);
    ((total + per - 1) / per).max(1)
}

/// PNG mode: har frame ek PNG file (M1/M2 jaisa).
fn render_job_frames(
    job: &Job,
    cfg: &Config,
    assets_root: &Path,
    out_dir: &Path,
) -> Result<(), String> {
    render_frames_with(job, cfg, assets_root, |frame_idx, frame| {
        let out_path = out_dir.join(format!("frame_{:03}.png", frame_idx));
        save_png(frame, &out_path)
    })
}

/// Video mode: frames FFmpeg ko, har piece ki MP4 `out_dir` mein.
/// Piece band hote hi `on_piece(piece_number, mp4_path)` call hota hai.
/// Kitne pieces bane wo wapas deta hai.
pub(crate) fn render_job_pieces(
    job: &Job,
    cfg: &Config,
    assets_root: &Path,
    out_dir: &Path,
    on_piece: &mut dyn FnMut(u32, &Path) -> Result<(), String>,
) -> Result<u32, String> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("create out_dir {}: {e}", out_dir.display()))?;
    let (vid_w, vid_h) = effective_video(job, cfg);
    let fps = job.video.fps.unwrap_or(cfg.fps);
    let piece_sec = job.video.piece_sec.unwrap_or(cfg.piece_sec);

    let mut sink = PieceSink::new(cfg, out_dir, vid_w, vid_h, fps, piece_sec);
    render_frames_with(job, cfg, assets_root, |frame_idx, frame| {
        sink.push_frame(frame_idx, &frame.rgba, &mut *on_piece)
    })?;
    let n = sink.finish(&mut *on_piece)?;
    if n == 0 {
        return Err("job has no frames (duration_sec too small)".to_string());
    }
    Ok(n)
}

/// Frames ko pieces mein baantta hai: har piece ke liye ek PieceEncoder.
struct PieceSink {
    cfg: Config,
    out_dir: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    piece_sec: f32,
    current: Option<(u32, PathBuf, encoder::PieceEncoder)>,
    pieces_done: u32,
}

impl PieceSink {
    fn new(cfg: &Config, out_dir: &Path, width: u32, height: u32, fps: u32, piece_sec: f32) -> Self {
        // FFmpeg ko job ka fps chahiye, config ka nahi.
        let mut c = *cfg;
        c.fps = fps;
        Self {
            cfg: c,
            out_dir: out_dir.to_path_buf(),
            width,
            height,
            fps,
            piece_sec,
            current: None,
            pieces_done: 0,
        }
    }

    /// Chalu piece band karo, aur on_piece se batao.
    fn close_current(
        &mut self,
        on_piece: &mut dyn FnMut(u32, &Path) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some((piece, path, enc)) = self.current.take() {
            enc.finish().map_err(|e| format!("piece {}: {}", piece, e))?;
            self.pieces_done += 1;
            on_piece(piece, &path)?;
        }
        Ok(())
    }

    fn push_frame(
        &mut self,
        frame_idx: u32,
        rgba: &[u8],
        on_piece: &mut dyn FnMut(u32, &Path) -> Result<(), String>,
    ) -> Result<(), String> {
        let piece = encoder::piece_for_frame(frame_idx, self.fps, self.piece_sec);
        let need_new = match &self.current {
            Some((p, _, _)) => *p != piece,
            None => true,
        };
        if need_new {
            self.close_current(&mut *on_piece)?;
            let path = self.out_dir.join(encoder::piece_file_name(piece));
            let enc = encoder::PieceEncoder::start(&self.cfg, self.width, self.height, &path)?;
            self.current = Some((piece, path, enc));
        }
        if let Some((_, _, enc)) = self.current.as_mut() {
            enc.write_frame(rgba)?;
        }
        Ok(())
    }

    /// Aakhri piece band karo. Kitne pieces bane wo wapas.
    fn finish(
        mut self,
        on_piece: &mut dyn FnMut(u32, &Path) -> Result<(), String>,
    ) -> Result<u32, String> {
        self.close_current(&mut *on_piece)?;
        Ok(self.pieces_done)
    }
}

/// Saare frames render karta hai; har frame ban ke `on_frame` ko milta hai
/// (PNG save ya FFmpeg pipe). Frame buffer har baar reuse hota hai (§11).
fn render_frames_with<F>(
    job: &Job,
    cfg: &Config,
    assets_root: &Path,
    mut on_frame: F,
) -> Result<(), String>
where
    F: FnMut(u32, &FrameBuffer) -> Result<(), String>,
{
    let (vid_w, vid_h) = effective_video(job, cfg);
    let fps = job.video.fps.unwrap_or(cfg.fps);
    let total_frames = total_frames(job, cfg);

    let mut loader = AssetLoader::new(assets_root.to_path_buf(), cfg.asset_cache_bytes);
    let bg = loader
        .load(&job.background.asset, 1.0)
        .map_err(|e| format!("background '{}': {e}", job.background.asset))?;

    let mut frame = FrameBuffer::new(vid_w, vid_h);
    let mut manifest_cache: HashMap<String, SkinManifest> = HashMap::new();
    let mut image_cache: HashMap<String, HashMap<String, Arc<CachedImage>>> = HashMap::new();

    for ch in &job.characters {
        if !manifest_cache.contains_key(&ch.skin) {
            let m = load_skin_manifest(assets_root, &ch.skin)?;
            manifest_cache.insert(ch.skin.clone(), m);
        }
        let manifest = &manifest_cache[&ch.skin];
        let images = load_character_images(manifest, ch, &mut loader)?;
        image_cache.insert(ch.id.clone(), images);
    }

    for frame_idx in 0..total_frames {
        let t = frame_idx as f32 / fps as f32;

        let cam = resolve_camera(job, t);
        let crop = camera::window(&cam, (bg.width, bg.height), (vid_w, vid_h));
        let mut poses: Vec<ResolvedPose> = Vec::with_capacity(job.characters.len());

        for ch in &job.characters {
            let manifest = &manifest_cache[&ch.skin];
            let images = &image_cache[&ch.id];

            let state = timeline::get_character_state(job, &ch.id, (ch.x, ch.y), t);

            // M3: Face State resolve karo
            let face_state = face::resolve_face_state(job, &ch.id, t);
            let face_assets = face::resolve_face(&ch.skin, &face_state);

            // Clone image map aur face images dynamically inject karo
            let mut current_images = images.clone();
            if manifest.parts.contains_key("face_eyes") {
                let img = loader
                    .load(&face_assets.eyes, ch.height)
                    .unwrap_or_else(|_| dummy_image());
                current_images.insert("FACE_EYES".to_string(), img);
            }
            if manifest.parts.contains_key("face_mouth") {
                let img = loader
                    .load(&face_assets.mouth, ch.height)
                    .unwrap_or_else(|_| dummy_image());
                current_images.insert("FACE_MOUTH".to_string(), img);
            }

            let pose = resolve_animated_pose(
                manifest,
                state.pos,
                &current_images,
                state.action,
                state.walk_phase,
                state.action_elapsed,
            )
            .map_err(|e| format!("character '{}': {e}", ch.id))?;

            poses.push(pose);
        }

        let merged = merge_poses(poses);
        compose_frame(&mut frame, &bg, &crop, &merged);
        on_frame(frame_idx, &frame)?;
    }

    Ok(())
}

fn load_skin_manifest(assets_root: &Path, skin: &str) -> Result<SkinManifest, String> {
    let path = assets_root.join(skin).join("skin.json");
    let file = std::fs::File::open(&path).map_err(|e| format!("open skin '{}': {e}", path.display()))?;
    parse_skin(file).map_err(|e| format!("skin json {}: {e}", path.display()))
}

fn load_character_images(
    manifest: &SkinManifest,
    ch: &Character,
    loader: &mut AssetLoader,
) -> Result<HashMap<String, Arc<CachedImage>>, String> {
    let mut out: HashMap<String, Arc<CachedImage>> = HashMap::new();
    for part in manifest.parts.values() {
        if part.asset == "FACE_EYES" || part.asset == "FACE_MOUTH" { continue; }
        if !out.contains_key(&part.asset) {
            let img = loader.load(&part.asset, ch.height).map_err(|e| format!("char '{}' asset '{}': {e}", ch.id, part.asset))?;
            out.insert(part.asset.clone(), img);
        }
    }
    Ok(out)
}

fn merge_poses(poses: Vec<ResolvedPose>) -> ResolvedPose {
    let mut parts: Vec<ResolvedPart> = poses.into_iter().flat_map(|p| p.parts).collect();
    parts.sort_by_key(|p| p.z);
    ResolvedPose { parts }
}

fn effective_video(job: &Job, cfg: &Config) -> (u32, u32) {
    (job.video.w.unwrap_or(cfg.video_width), job.video.h.unwrap_or(cfg.video_height))
}

fn save_png(frame: &FrameBuffer, path: &Path) -> Result<(), String> {
    let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone()).ok_or_else(|| "internal: frame buffer size mismatch".to_string())?;
    img.save(path).map_err(|e| format!("png write {}: {e}", path.display()))
}

fn resolve_camera(job: &Job, t: f32) -> Camera {
    let mut pan_x = 0.0;
    let mut pan_y = 0.0;
    for ev in &job.timeline {
        if let Some(cam_ev) = &ev.camera {
            if t > ev.t {
                let dur = cam_ev.dur.max(0.001);
                let progress = ((t - ev.t) / dur).clamp(0.0, 1.0);
                let dist = cam_ev.amount * progress;

                match cam_ev.move_dir.as_str() {
                    "E" => pan_x += dist,
                    "W" => pan_x -= dist,
                    "S" => pan_y += dist,
                    "N" => pan_y -= dist,
                    _ => {}
                }
            }
        }
    }
    Camera { pan_x, pan_y }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_part(name: &str, z: i32) -> ResolvedPart {
        ResolvedPart {
            name: name.to_string(),
            z,
            image: Arc::new(CachedImage { rgba: vec![], width: 0, height: 0, src_width: 0, src_height: 0 }),
            top_left: (0.0, 0.0),
            world_pivot: (0.0, 0.0),
            angle_rad: 0.0,
        }
    }

    fn dummy_job() -> Job {
        Job {
            job_id: "t".to_string(),
            video: types::VideoOpts::default(),
            background: types::Background { asset: "bg".to_string(), w: 100, h: 100 },
            characters: vec![],
            timeline: vec![],
        }
    }

    #[test]
    fn merge_two_poses_interleaves_by_z() {
        let a = ResolvedPose { parts: vec![dummy_part("a.body", 3), dummy_part("a.p6", 4)] };
        let b = ResolvedPose { parts: vec![dummy_part("b.body", 5), dummy_part("b.p6", 6)] };
        let merged = merge_poses(vec![a, b]);
        let names: Vec<&str> = merged.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["a.body", "a.p6", "b.body", "b.p6"]);
    }

    #[test]
    fn merge_tie_on_z_keeps_character_order() {
        let a = ResolvedPose { parts: vec![dummy_part("a.body", 3)] };
        let b = ResolvedPose { parts: vec![dummy_part("b.body", 3)] };
        let merged = merge_poses(vec![a, b]);
        let names: Vec<&str> = merged.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["a.body", "b.body"]);
    }

    #[test]
    fn effective_video_uses_job_when_present() {
        let mut job = dummy_job();
        job.video.w = Some(1080);
        job.video.h = Some(1920);
        let cfg = Config::default();
        assert_eq!(effective_video(&job, &cfg), (1080, 1920));
    }

    #[test]
    fn camera_pan_and_tilt_resolve() {
        let mut job = dummy_job();
        job.timeline.push(types::TimelineEvent {
            t: 0.0, char: None, action: None, to: None, dur: None, face: None,
            camera: Some(types::CameraEvent { move_dir: "E".to_string(), amount: 100.0, dur: 2.0 }),
        });
        job.timeline.push(types::TimelineEvent {
            t: 0.0, char: None, action: None, to: None, dur: None, face: None,
            camera: Some(types::CameraEvent { move_dir: "S".to_string(), amount: 50.0, dur: 1.0 }),
        });

        let cam1 = resolve_camera(&job, 1.0); // Mid-pan
        assert_eq!(cam1.pan_x, 50.0);
        assert_eq!(cam1.pan_y, 50.0);

        let cam2 = resolve_camera(&job, 3.0); // End-pan
        assert_eq!(cam2.pan_x, 100.0);
        assert_eq!(cam2.pan_y, 50.0);
    }

    #[test]
    fn count_pieces_full_and_partial() {
        let cfg = Config::default();
        let mut job = dummy_job();
        // 20 sec * 15 fps = 300 frames, 75 per piece -> 4 pieces.
        job.video.duration_sec = Some(20.0);
        assert_eq!(count_pieces(&job, &cfg), 4);
        // 6 sec = 90 frames -> 2 pieces (75 + 15).
        job.video.duration_sec = Some(6.0);
        assert_eq!(count_pieces(&job, &cfg), 2);
        // Job ka apna piece_sec.
        job.video.duration_sec = Some(10.0);
        job.video.piece_sec = Some(2.0);
        assert_eq!(count_pieces(&job, &cfg), 5);
    }

    #[test]
    fn piece_sink_splits_frames_into_pieces() {
        let cfg = Config::default();
        if !encoder::ffmpeg_available(cfg.ffmpeg_path) {
            eprintln!("ffmpeg not found, skipping piece_sink_splits_frames_into_pieces");
            return;
        }
        let dir = std::env::temp_dir().join(format!("lkj_sink_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (64u32, 64u32);
        // fps 10, piece 1 sec = 10 frames per piece; 25 frames -> 3 pieces.
        let mut sink = PieceSink::new(&cfg, &dir, w, h, 10, 1.0);
        let frame = vec![128u8; (w * h * 4) as usize];
        let mut seen: Vec<u32> = Vec::new();
        {
            let mut on_piece = |piece: u32, path: &Path| -> Result<(), String> {
                assert!(path.exists(), "piece file missing: {}", path.display());
                seen.push(piece);
                Ok(())
            };
            for i in 0..25u32 {
                sink.push_frame(i, &frame, &mut on_piece).unwrap();
            }
            let n = sink.finish(&mut on_piece).unwrap();
            assert_eq!(n, 3);
        }
        assert_eq!(seen, vec![0, 1, 2]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
