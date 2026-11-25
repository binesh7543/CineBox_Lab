//! M1 end-to-end test (§14): ek job JSON -> PNG, pixel-verified.
//! M2 Update: Engine ab single PNG ke bajaye frames ka directory banata hai.
//! Tests ko is naye behavior ke hisaab se update kiya gaya hai.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_lkj-engine");

// ---------------- helpers ----------------

fn fresh_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join(format!("lkj-m1-{}-{}-{}", tag, std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn write_solid_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut img = image::RgbaImage::new(w, h);
    for (_, _, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba(rgba);
    }
    img.save(path).expect("write png");
}

fn write_skin(root: &Path, skin_name: &str, skin_json: &str) {
    let dir = root.join(skin_name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("skin.json"), skin_json).unwrap();
}

fn write_job(path: &Path, job_json: &str) {
    std::fs::write(path, job_json).unwrap();
}

fn run_engine(job: &Path, out_dir: &Path, assets: &Path) -> std::process::Output {
    Command::new(BIN)
        .arg(job)
        .arg(out_dir)
        .arg(assets)
        .output()
        .expect("spawn lkj-engine")
}

fn read_png(path: &Path) -> image::RgbaImage {
    image::open(path)
        .expect("open png")
        .to_rgba8()
}

fn assert_px(img: &image::RgbaImage, x: u32, y: u32, expected: [u8; 4]) {
    let got = img.get_pixel(x, y).0;
    assert_eq!(
        got, expected,
        "pixel ({x},{y}) expected {expected:?}, got {got:?}"
    );
}

const OGGY_SKIN: &str = r#"{
  "skin": "oggy", "base_height": 400,
  "parts": {
    "body": { "asset": "oggy/body",    "pivot": {"x": 20, "y": 20}, "z": 3 },
    "p10":  { "asset": "oggy/part_10", "pivot": {"x": 5,  "y": 5 }, "z": 4,
              "parent": "body", "attach": {"x": 10, "y": 10} }
  }
}"#;

fn write_oggy_assets(root: &Path) {
    write_skin(root, "oggy", OGGY_SKIN);
    write_solid_png(&root.join("oggy/body.png"),    40, 40, [255, 0,   0,   255]);
    write_solid_png(&root.join("oggy/part_10.png"), 10, 10, [0,   255, 0,   255]);
}

fn write_bg(root: &Path) {
    write_solid_png(&root.join("bg.png"), 200, 200, [0, 0, 255, 255]);
}

// ---------------- tests ----------------

#[test]
fn single_character_m1_full_flow() {
    let dir = fresh_dir("single");
    write_oggy_assets(&dir);
    write_bg(&dir);

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_single",
          "video": { "w": 200, "h": 200, "fps": 15, "duration_sec": 0.1 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": [
            { "id": "c1", "skin": "oggy", "height": 1.0, "x": 100, "y": 100 }
          ]
        }"#,
    );

    let out_dir = dir.join("out_frames");
    let output = run_engine(&job_path, &out_dir, &dir);
    assert!(
        output.status.success(),
        "binary failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Engine M2 ab ek folder banata hai, usme se pehla frame padho
    let img = read_png(&out_dir.join("frame_000.png"));
    assert_eq!((img.width(), img.height()), (200, 200), "frame size");

    assert_px(&img, 0, 0, [0, 0, 255, 255]);
    assert_px(&img, 79, 79, [0, 0, 255, 255]); 
    assert_px(&img, 120, 120, [0, 0, 255, 255]);

    assert_px(&img, 80, 80, [255, 0, 0, 255]);
    assert_px(&img, 119, 119, [255, 0, 0, 255]);
    assert_px(&img, 100, 100, [255, 0, 0, 255]);

    assert_px(&img, 85, 85, [0, 255, 0, 255]);
    assert_px(&img, 94, 94, [0, 255, 0, 255]);
    assert_px(&img, 95, 95, [255, 0, 0, 255]);
}

#[test]
fn character_height_scales_all_parts() {
    let dir = fresh_dir("height");
    write_oggy_assets(&dir);
    write_bg(&dir);

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_height",
          "video": { "w": 200, "h": 200, "fps": 15, "duration_sec": 0.1 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": [
            { "id": "c1", "skin": "oggy", "height": 0.5, "x": 100, "y": 100 }
          ]
        }"#,
    );

    let out_dir = dir.join("half_frames");
    let output = run_engine(&job_path, &out_dir, &dir);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let img = read_png(&out_dir.join("frame_000.png"));
    assert_px(&img, 100, 100, [255, 0, 0, 255]);
    assert_px(&img, 89, 89, [0, 0, 255, 255]);
    assert_px(&img, 90, 90, [255, 0, 0, 255]);
    assert_px(&img, 109, 109, [255, 0, 0, 255]);
    assert_px(&img, 110, 110, [0, 0, 255, 255]);
    assert_px(&img, 93, 93, [0, 255, 0, 255]);
}

#[test]
fn multiple_characters_render_together() {
    let dir = fresh_dir("multi");
    write_oggy_assets(&dir);
    write_bg(&dir);

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_multi",
          "video": { "w": 200, "h": 200, "fps": 15, "duration_sec": 0.1 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": [
            { "id": "c1", "skin": "oggy", "height": 1.0, "x":  60, "y":  60 },
            { "id": "c2", "skin": "oggy", "height": 1.0, "x": 140, "y": 140 }
          ]
        }"#,
    );

    let out_dir = dir.join("multi_frames");
    let output = run_engine(&job_path, &out_dir, &dir);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let img = read_png(&out_dir.join("frame_000.png"));
    assert_px(&img, 60, 60, [255, 0, 0, 255]);
    assert_px(&img, 140, 140, [255, 0, 0, 255]);
    assert_px(&img, 100, 100, [0, 0, 255, 255]);
}

#[test]
fn missing_skin_json_fails_with_message() {
    let dir = fresh_dir("noskin");
    write_solid_png(&dir.join("oggy/body.png"), 10, 10, [255, 0, 0, 255]);
    write_solid_png(&dir.join("oggy/part_10.png"), 5, 5, [0, 255, 0, 255]);
    write_bg(&dir);

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_noskin",
          "video": { "w": 200, "h": 200 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": [
            { "id": "c1", "skin": "oggy", "height": 1.0, "x": 100, "y": 100 }
          ]
        }"#,
    );

    let out_dir = dir.join("fail_frames");
    let output = run_engine(&job_path, &out_dir, &dir);
    assert!(!output.status.success(), "missing skin.json must fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("skin") || stderr.contains("open"),
        "stderr should mention skin/open: {stderr}"
    );
    // M2: Check karo ki koi frame generate nahi hua
    assert!(!out_dir.join("frame_000.png").exists(), "no output on failure");
}

#[test]
fn background_only_no_characters() {
    let dir = fresh_dir("bonly");
    write_bg(&dir);

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_bgonly",
          "video": { "w": 200, "h": 200, "fps": 15, "duration_sec": 0.1 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": []
        }"#,
    );

    let out_dir = dir.join("bg_frames");
    let output = run_engine(&job_path, &out_dir, &dir);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let img = read_png(&out_dir.join("frame_000.png"));
    for (x, y) in [(0u32, 0u32), (100, 100), (199, 199)] {
        assert_px(&img, x, y, [0, 0, 255, 255]);
    }
}

#[test]
fn missing_background_fails_cleanly() {
    let dir = fresh_dir("nobg");

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_nobg",
          "video": { "w": 200, "h": 200 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": []
        }"#,
    );

    let out_dir = dir.join("nobg_frames");
    let output = run_engine(&job_path, &out_dir, &dir);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("background") || stderr.contains("not found"),
        "stderr: {stderr}"
    );
    assert!(!out_dir.join("frame_000.png").exists());
}
