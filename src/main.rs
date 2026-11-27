//! M2 CLI: JSON -> multiple PNG frames.
//!
//! Usage:
//!     lkj-engine <job.json> <out_dir> [assets_root]
//!
//! M3 Update:
//!   - Camera pan/tilt timeline events resolve kiye gaye hain.
//!   - Har frame par camera ki position calculate hoti hai aur `compositor` ko di jati hai.

mod assets;
mod camera;
mod compositor;
mod config;
mod face;
mod json_stream;
mod skeleton;
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
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: {} <job.json> <out_dir> [assets_root]", args[0]);
        std::process::exit(2);
    }
    let job_path = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    let assets_root = args.get(3).cloned().unwrap_or_else(|| ".".to_string());

    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("create out_dir {}: {e}", out_dir.display()))?;

    let cfg = Config::default();
    cfg.validate();

    let job = load_job(&job_path)?;
    render_job_frames(&job, &cfg, Path::new(&assets_root), &out_dir)?;
    Ok(())
}

// ---------------- Steps ----------------

fn load_job(path: &Path) -> Result<Job, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("open job {}: {e}", path.display()))?;
    parse_job(file).map_err(|e| format!("job json {}: {e}", path.display()))
}

fn render_job_frames(
    job: &Job,
    cfg: &Config,
    assets_root: &Path,
    out_dir: &Path,
) -> Result<(), String> {
    let (vid_w, vid_h) = effective_video(job, cfg);
    let fps = job.video.fps.unwrap_or(cfg.fps);
    let duration = job.video.duration_sec.unwrap_or(1.0);
    let total_frames = (duration * fps as f32).ceil() as u32;

    let mut loader = AssetLoader::new(assets_root.to_path_buf(), cfg.asset_cache_bytes);

    let bg = loader
        .load(&job.background.asset, 1.0)
        .map_err(|e| format!("background '{}': {e}", job.background.asset))?;

    let mut frame = FrameBuffer::new(vid_w, vid_h);

    // Cache manifests and images per skin to avoid reloading per frame
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
        
        // M3: Camera state resolve karo frame `t` ke liye
        let cam = resolve_camera(job, t);
        let crop = camera::window(&cam, (bg.width, bg.height), (vid_w, vid_h));

        let mut poses: Vec<ResolvedPose> = Vec::with_capacity(job.characters.len());

        for ch in &job.characters {
            let manifest = &manifest_cache[&ch.skin];
            let images = &image_cache[&ch.id];

            // 1. Get interpolated state from timeline
            let state = timeline::get_character_state(job, &ch.id, (ch.x, ch.y), t);

            // 2. Resolve pose with action/rotation
            let pose = resolve_animated_pose(
                manifest,
                state.pos,
                images,
                state.action,
                state.walk_phase,
                state.action_elapsed,
            )
            .map_err(|e| format!("character '{}': {e}", ch.id))?;

            poses.push(pose);
        }

        let merged = merge_poses(poses);
        compose_frame(&mut frame, &bg, &crop, &merged);

        let out_path = out_dir.join(format!("frame_{:03}.png", frame_idx));
        save_png(&frame, &out_path)?;
    }

    Ok(())
}

fn load_skin_manifest(assets_root: &Path, skin: &str) -> Result<SkinManifest, String> {
    let path = assets_root.join(skin).join("skin.json");
    let file = std::fs::File::open(&path)
        .map_err(|e| format!("open skin '{}': {e}", path.display()))?;
    parse_skin(file).map_err(|e| format!("skin json {}: {e}", path.display()))
}

fn load_character_images(
    manifest: &SkinManifest,
    ch: &Character,
    loader: &mut AssetLoader,
) -> Result<HashMap<String, Arc<CachedImage>>, String> {
    let mut out: HashMap<String, Arc<CachedImage>> = HashMap::new();
    for part in manifest.parts.values() {
        if !out.contains_key(&part.asset) {
            let img = loader
                .load(&part.asset, ch.height)
                .map_err(|e| format!("char '{}' asset '{}': {e}", ch.id, part.asset))?;
            out.insert(part.asset.clone(), img);
        }
    }
    Ok(out)
}

fn merge_poses(poses: Vec<ResolvedPose>) -> ResolvedPose {
    let mut parts: Vec<ResolvedPart> = poses.into_iter().flat_map(|p| p.parts).collect();
    // Stable sort ensures character order is preserved on z-tie
    parts.sort_by_key(|p| p.z);
    ResolvedPose { parts }
}

fn effective_video(job: &Job, cfg: &Config) -> (u32, u32) {
    (
        job.video.w.unwrap_or(cfg.video_width),
        job.video.h.unwrap_or(cfg.video_height),
    )
}

fn save_png(frame: &FrameBuffer, path: &Path) -> Result<(), String> {
    let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
        .ok_or_else(|| "internal: frame buffer size mismatch".to_string())?;
    img.save(path)
        .map_err(|e| format!("png write {}: {e}", path.display()))
}

// M3: Camera events (E, W, N, S) resolve karne ka logic
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

// ---------------- Unit tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_part(name: &str, z: i32) -> ResolvedPart {
        ResolvedPart {
            name: name.to_string(),
            z,
            image: Arc::new(CachedImage {
                rgba: vec![],
                width: 0,
                height: 0,
                src_width: 0,
                src_height: 0,
            }),
            top_left: (0.0, 0.0),
            world_pivot: (0.0, 0.0),
            angle_rad: 0.0,
        }
    }

    fn dummy_job() -> Job {
        Job {
            job_id: "t".to_string(),
            video: types::VideoOpts::default(),
            background: types::Background {
                asset: "bg".to_string(),
                w: 100,
                h: 100,
            },
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
            t: 0.0,
            char: None,
            action: None,
            to: None,
            dur: None,
            face: None,
            camera: Some(types::CameraEvent { move_dir: "E".to_string(), amount: 100.0, dur: 2.0 }),
        });
        job.timeline.push(types::TimelineEvent {
            t: 0.0,
            char: None,
            action: None,
            to: None,
            dur: None,
            face: None,
            camera: Some(types::CameraEvent { move_dir: "S".to_string(), amount: 50.0, dur: 1.0 }),
        });

        let cam1 = resolve_camera(&job, 1.0); // Mid-pan
        assert_eq!(cam1.pan_x, 50.0); // 100 * (1/2)
        assert_eq!(cam1.pan_y, 50.0); // 50 * (1/1)

        let cam2 = resolve_camera(&job, 3.0); // End-pan
        assert_eq!(cam2.pan_x, 100.0);
        assert_eq!(cam2.pan_y, 50.0);
    }
                  }
