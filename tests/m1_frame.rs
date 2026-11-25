//! M1 end-to-end test (§14): ek job JSON -> PNG, pixel-verified.
//!
//! Test strategy:
//!   1. Temp dir mein skin.json + PNGs + job.json likho (user point 3:
//!      PNGs runtime pe generate, sirf tests mein — production code koi
//!      image nahi banata).
//!   2. `lkj-engine <job.json> <out.png> <assets_root>` ko subprocess mein
//!      chalao (koi lib target nahi hai, to ye hi tareeka hai).
//!   3. Output PNG load karke pixel assertions.
//!
//! **User B pin:** `character.x, y` = **body.pivot ki screen position**.
//! `single_character_m1_full_flow` isko `(100,100)` pe body red pixel se
//! pin karta hai. Zameen anchor (pair ke neeche) M2 mein — tab ye assertion
//! __na badle__, kyunki character ke x/y ka matlab wahi rahega; sirf
//! skeleton `body.pivot` ke upar ground offset laayega.
//!
//! Runtime PNGs ke liye `image` crate use hota hai jo already dep hai
//! (image = png feature). `env!("CARGO_BIN_EXE_lkj-engine")` Cargo ka
//! built-in integration-test path hai — agar Cargo.toml mein bin ka naam
//! badle, yahan bhi badalna hai.
//!
//! Temp dirs `/tmp/lkj-m1-*` mein rehti hain (best-effort cleanup next run
//! par same name milne pe), taaki test fail ho to debug artifacts pade rahein.

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

/// Solid-colour PNG (runtime — test-only per user point 3).
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

fn run_engine(job: &Path, out: &Path, assets: &Path) -> std::process::Output {
    Command::new(BIN)
        .arg(job)
        .arg(out)
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

/// Common rig: body 40x40 pivot (20,20) + p10 10x10 pivot (5,5) attach (10,10).
/// Body z=3 < p10 z=4. Height=1.0 ke saath, char (100,100) pe:
///   body : top_left (80,80)  covers x,y ∈ [80, 120)
///   p10  : world (90,90) tl (85,85) covers [85, 95)
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
    write_solid_png(&root.join("oggy/body.png"),    40, 40, [255, 0,   0,   255]); // red
    write_solid_png(&root.join("oggy/part_10.png"), 10, 10, [0,   255, 0,   255]); // green
}

fn write_bg(root: &Path) {
    write_solid_png(&root.join("bg.png"), 200, 200, [0, 0, 255, 255]); // blue
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
          "video": { "w": 200, "h": 200 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": [
            { "id": "c1", "skin": "oggy", "height": 1.0, "x": 100, "y": 100 }
          ]
        }"#,
    );

    let out = dir.join("out.png");
    let output = run_engine(&job_path, &out, &dir);
    assert!(
        output.status.success(),
        "binary failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let img = read_png(&out);
    assert_eq!((img.width(), img.height()), (200, 200), "frame size");

    // Background fills everywhere char nahi hai.
    assert_px(&img, 0, 0, [0, 0, 255, 255]);
    assert_px(&img, 79, 79, [0, 0, 255, 255]);   // just outside body top_left
    assert_px(&img, 120, 120, [0, 0, 255, 255]); // just past body bottom-right

    // Body top-left and bottom-right corners visible.
    assert_px(&img, 80, 80, [255, 0, 0, 255]);
    assert_px(&img, 119, 119, [255, 0, 0, 255]);

    // **User B pin:** body pivot at (100,100) → red body pixel there.
    assert_px(&img, 100, 100, [255, 0, 0, 255]);

    // p10 z=4 > body z=3 → green on top of body.
    assert_px(&img, 85, 85, [0, 255, 0, 255]);
    assert_px(&img, 90, 90, [0, 255, 0, 255]);
    assert_px(&img, 94, 94, [0, 255, 0, 255]);
    // Just outside p10 (but inside body) → red.
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
          "video": { "w": 200, "h": 200 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": [
            { "id": "c1", "skin": "oggy", "height": 0.5, "x": 100, "y": 100 }
          ]
        }"#,
    );

    let out = dir.join("half.png");
    let output = run_engine(&job_path, &out, &dir);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let img = read_png(&out);
    // body 40x40 src → 20x20 disp; pivot (20,20)→(10,10); top_left (90,90);
    // covers [90, 110). p10 10x10→5x5; world_pivot (95,95); tl (93,93) after
    // rounding; covers [93, 98).

    // User B still holds: body.pivot screen pos == character.x,y.
    assert_px(&img, 100, 100, [255, 0, 0, 255]);

    // Body bounds (scaled).
    assert_px(&img, 89, 89, [0, 0, 255, 255]);  // just outside left/top
    assert_px(&img, 90, 90, [255, 0, 0, 255]);  // body top-left
    assert_px(&img, 109, 109, [255, 0, 0, 255]); // body bottom-right
    assert_px(&img, 110, 110, [0, 0, 255, 255]); // outside

    // p10 scaled and z-ordered.
    assert_px(&img, 93, 93, [0, 255, 0, 255]);
    assert_px(&img, 97, 97, [0, 255, 0, 255]);
    assert_px(&img, 98, 98, [255, 0, 0, 255]); // p10 ends before this
}

#[test]
fn multiple_characters_render_together() {
    // §2: "Ek screen pe kai characters ek saath aa sakte hain."
    let dir = fresh_dir("multi");
    write_oggy_assets(&dir);
    write_bg(&dir);

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_multi",
          "video": { "w": 200, "h": 200 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": [
            { "id": "c1", "skin": "oggy", "height": 1.0, "x":  60, "y":  60 },
            { "id": "c2", "skin": "oggy", "height": 1.0, "x": 140, "y": 140 }
          ]
        }"#,
    );

    let out = dir.join("multi.png");
    let output = run_engine(&job_path, &out, &dir);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let img = read_png(&out);

    // c1 body [40, 80) — pivot at (60,60).
    assert_px(&img, 60, 60, [255, 0, 0, 255]);
    assert_px(&img, 40, 40, [255, 0, 0, 255]);
    assert_px(&img, 79, 79, [255, 0, 0, 255]);

    // c2 body [120, 160) — pivot at (140,140).
    assert_px(&img, 140, 140, [255, 0, 0, 255]);
    assert_px(&img, 120, 120, [255, 0, 0, 255]);
    assert_px(&img, 159, 159, [255, 0, 0, 255]);

    // Middle point is bg (bodies don't overlap).
    assert_px(&img, 100, 100, [0, 0, 255, 255]);
}

#[test]
fn missing_skin_json_fails_with_message() {
    let dir = fresh_dir("noskin");
    // skin.json jaan-boojh ke nahi likhi; sirf PNGs hain.
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

    let out = dir.join("out.png");
    let output = run_engine(&job_path, &out, &dir);
    assert!(!output.status.success(), "missing skin.json must fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("skin") || stderr.contains("open"),
        "stderr should mention skin/open: {stderr}"
    );
    assert!(!out.exists(), "no output on failure");
}

#[test]
fn background_only_no_characters() {
    let dir = fresh_dir("bonly");
    write_bg(&dir);
    // Koi skin assets likhe hi nahi — engine ko character-free job chalna chahiye.

    let job_path = dir.join("job.json");
    write_job(
        &job_path,
        r#"{
          "job_id": "m1_bgonly",
          "video": { "w": 200, "h": 200 },
          "background": { "asset": "bg", "w": 200, "h": 200 },
          "characters": []
        }"#,
    );

    let out = dir.join("bg.png");
    let output = run_engine(&job_path, &out, &dir);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let img = read_png(&out);
    for (x, y) in [(0u32, 0u32), (100, 100), (199, 199)] {
        assert_px(&img, x, y, [0, 0, 255, 255]);
    }
}

#[test]
fn missing_background_fails_cleanly() {
    let dir = fresh_dir("nobg");
    // bg.png nahi likhi.

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

    let out = dir.join("out.png");
    let output = run_engine(&job_path, &out, &dir);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("background") || stderr.contains("not found"),
        "stderr: {stderr}"
    );
    assert!(!out.exists());
              }
