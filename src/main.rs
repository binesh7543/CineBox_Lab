//! M1 CLI: ek JSON se ek PNG frame.
//!
//! Usage:
//!     render_frame <job.json> <out.png> [assets_root]
//!
//! assets_root default = ".". Asset paths `<root>/<skin>/skin.json`,
//! `<root>/<asset>.png` — §9 ke naming ("oggy/body", "oggy/part_6") se.
//!
//! **M1 scope:**
//!   - 1 background + N characters, default pose (koi action/camera nahi)
//!   - `job.video.w/h` (fallback §15 defaults) — video frame size
//!   - Background actual loaded image ke dims crop math ke liye use hote hain
//!     (JSON ka `bg.w/h` metadata hai — loaded PNG truth hai)
//!   - `character.x, y` = body pivot screen pos (user B assumption)
//!   - Multiple characters: merged into a single z-sorted part list,
//!     character order preserved on z-tie (character[0] behind character[1])
//!   - `face.rs` M1 flow mein invoke nahi hota (M2 mein actions ke saath)
//!   - `fps`/`duration_sec`/`piece_sec` M4 tak unused
//!
//! Ye module wiring hai; asli tests `tests/m1_frame.rs` mein binary invocation
//! se, plus 2 chhote unit tests yahin niche (pure helpers).

mod assets;
mod camera;
mod compositor;
mod config;
mod face;
mod json_stream;
mod skeleton;
mod types;

use assets::{AssetLoader, CachedImage};
use camera::Camera;
use compositor::{compose_frame, FrameBuffer};
use config::Config;
use json_stream::{parse_job, parse_skin};
use skeleton::{resolve_pose, ResolvedPart, ResolvedPose};
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
        eprintln!("usage: {} <job.json> <out.png> [assets_root]", args[0]);
        std::process::exit(2);
    }
    let job_path = PathBuf::from(&args[1]);
    let out_path = PathBuf::from(&args[2]);
    let assets_root = args.get(3).cloned().unwrap_or_else(|| ".".to_string());

    let cfg = Config::default();
    cfg.validate();

    let job = load_job(&job_path)?;
    let frame = render_job(&job, &cfg, Path::new(&assets_root))?;
    save_png(&frame, &out_path)?;
    Ok(())
}

// ---------------- Steps (all pure-ish, easy to reason about) ----------------

fn load_job(path: &Path) -> Result<Job, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("open job {}: {e}", path.display()))?;
    parse_job(file).map_err(|e| format!("job json {}: {e}", path.display()))
}

fn render_job(job: &Job, cfg: &Config, assets_root: &Path) -> Result<FrameBuffer, String> {
    let (vid_w, vid_h) = effective_video(job, cfg);
    let mut loader = AssetLoader::new(assets_root.to_path_buf(), cfg.asset_cache_bytes);

    // §12: background bada image, camera crop window khisaye. Scale 1.0 —
    // background ko character.height se scale nahi karte.
    let bg = loader
        .load(&job.background.asset, 1.0)
        .map_err(|e| format!("background '{}': {e}", job.background.asset))?;

    // M1: camera static at top-left (Camera::STATIC). Pan/tilt math camera.rs
    // mein ready hai; sirf invocation badlega M3 mein.
    let crop = camera::window(&Camera::STATIC, (bg.width, bg.height), (vid_w, vid_h));

    let mut manifest_cache: HashMap<String, SkinManifest> = HashMap::new();
    let mut poses: Vec<ResolvedPose> = Vec::with_capacity(job.characters.len());
    for ch in &job.characters {
        if !manifest_cache.contains_key(&ch.skin) {
            let m = load_skin_manifest(assets_root, &ch.skin)?;
            manifest_cache.insert(ch.skin.clone(), m);
        }
        let manifest = &manifest_cache[&ch.skin];
        let images = load_character_images(manifest, ch, &mut loader)?;
        let pose = resolve_pose(manifest, (ch.x, ch.y), &images)
            .map_err(|e| format!("character '{}': {e}", ch.id))?;
        poses.push(pose);
    }

    let merged = merge_poses(poses);
    let mut frame = FrameBuffer::new(vid_w, vid_h);
    compose_frame(&mut frame, &bg, &crop, &merged);
    Ok(frame)
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
    // Har part ka apna asset §9 mein. Same asset (jaise body) multiple parts
    // mein nahi hota, par HashMap key collision-safe hai anyway.
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

/// Multiple `ResolvedPose` -> one `ResolvedPose`. Z-ascending. **Stable** sort:
/// same z pe character[0] ke parts character[1] ke parts se pehle (i.e.
/// character[0] behind), aur ek character ke andar original order preserved.
fn merge_poses(poses: Vec<ResolvedPose>) -> ResolvedPose {
    let mut parts: Vec<ResolvedPart> = poses.into_iter().flat_map(|p| p.parts).collect();
    parts.sort_by_key(|p| p.z);
    ResolvedPose { parts }
}

/// job.video.w/h (ya §15 default). Har axis independently overridable.
fn effective_video(job: &Job, cfg: &Config) -> (u32, u32) {
    (
        job.video.w.unwrap_or(cfg.video_width),
        job.video.h.unwrap_or(cfg.video_height),
    )
}

fn save_png(frame: &FrameBuffer, path: &Path) -> Result<(), String> {
    // RgbaImage::from_raw buffer ownership leta hai; ek clone (frame chhoti hoti
    // hai, aur CLI run one-shot hai). image 0.24/0.25 dono compatible.
    let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
        .ok_or_else(|| "internal: frame buffer size mismatch".to_string())?;
    img.save(path)
        .map_err(|e| format!("png write {}: {e}", path.display()))
}

// ---------------- Unit tests (pure helpers) ----------------
//
// Frame-level tests nahi yahan — wo tests/m1_frame.rs mein (binary spawn).
// Yahan sirf z-merge aur config override pin.

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
        }
    }

    #[test]
    fn merge_empty_is_empty() {
        let merged = merge_poses(vec![]);
        assert!(merged.parts.is_empty());
    }

    #[test]
    fn merge_single_pose_preserves_order() {
        let p = ResolvedPose {
            parts: vec![dummy_part("body", 3), dummy_part("p6", 4)],
        };
        let merged = merge_poses(vec![p]);
        let names: Vec<&str> = merged.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["body", "p6"]);
    }

    #[test]
    fn merge_two_poses_interleaves_by_z() {
        // Pose A: body z=3, p6 z=4.  Pose B: body z=5, p6 z=6.
        // Expected: a.body, a.p6, b.body, b.p6.
        let a = ResolvedPose {
            parts: vec![dummy_part("a.body", 3), dummy_part("a.p6", 4)],
        };
        let b = ResolvedPose {
            parts: vec![dummy_part("b.body", 5), dummy_part("b.p6", 6)],
        };
        let merged = merge_poses(vec![a, b]);
        let names: Vec<&str> = merged.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["a.body", "a.p6", "b.body", "b.p6"]);
    }

    #[test]
    fn merge_tie_on_z_keeps_character_order() {
        // Both z=3. Stable sort → a first, then b.
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
    fn effective_video_falls_back_to_config() {
        let job = dummy_job();
        let cfg = Config::default();
        assert_eq!(effective_video(&job, &cfg), (720, 1280));
    }

    #[test]
    fn effective_video_per_axis_override() {
        let mut job = dummy_job();
        job.video.w = Some(1080);
        // h missing -> cfg default
        let cfg = Config::default();
        assert_eq!(effective_video(&job, &cfg), (1080, 1280));
    }
      }
