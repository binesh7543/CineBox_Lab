//! Job JSON (§7) aur skin manifest (§9) ke data types.
//!
//! M1 scope: sirf wo fields jo ek frame render karne ke liye chahiye.
//! M2 update: Timeline events add kiye gaye hain (action, walk, face).
//! Face event parse ho raha hai, par M2 mein iska use nahi hoga (M3 ke liye).
//!
//! Assumption (user B): character.x / character.y = body part ke pivot ki screen
//! position. Zameen wala anchor (pair ke neeche) M2 mein dekhenge.

use serde::Deserialize;
use std::collections::HashMap;

/// Walk cycle ka period (seconds mein). Ek poora cycle (left + right step)
/// 1.0 second mein poora hoga. Isko timeline.rs aur skeleton.rs use karenge.
pub const WALK_CYCLE_PERIOD_SEC: f32 = 1.0;

/// 2D pixel coordinate. Pivot / attach dono isi type ke hain, lekin
/// **space alag hai**:
///   - `Part::pivot`  -> is part ki apni source image ke andar
///   - `Part::attach` -> parent ki source image ke andar
/// f32 isliye ki M2 scaling math fractional ho sakti hai.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };
    pub fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

// ---------------- Job (§7) ----------------

#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub job_id: String,
    #[serde(default)]
    pub video: VideoOpts,
    pub background: Background,
    pub characters: Vec<Character>,
    /// M2: Timeline events. Agar missing ho to empty vec (backward compatible with M1 jobs).
    #[serde(default)]
    pub timeline: Vec<TimelineEvent>,
}

/// §15 defaults Config se aate hain jab field missing ho — yahan har field
/// Option hai taaki caller tay kare (M1: main.rs Config ke saath merge karta hai).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VideoOpts {
    pub w: Option<u32>,
    pub h: Option<u32>,
    pub fps: Option<u32>,
    pub duration_sec: Option<f32>,
    pub piece_sec: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Background {
    pub asset: String,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Character {
    pub id: String,
    pub skin: String,
    #[serde(default = "default_height")]
    pub height: f32,
    pub x: i32,
    pub y: i32,
}

fn default_height() -> f32 {
    1.0
}

// ---------------- Timeline (§7 M2 Update) ----------------

/// Ek timeline event. M2 mein sirf action (idle, walk, wave) use hoga.
/// Baaki fields (face, camera) parse to honge, par ignore kiye jayenge.
#[derive(Debug, Clone, Deserialize)]
pub struct TimelineEvent {
    pub t: f32,
    /// Kis character ke liye hai. Camera events ke liye optional.
    #[serde(default)]
    pub char: Option<String>,
    /// Action name (e.g. "idle", "walk", "wave").
    #[serde(default)]
    pub action: Option<String>,
    /// Walk/Run ke liye destination.
    #[serde(default)]
    pub to: Option<Point>,
    /// Action ki duration (walk/run ke liye).
    #[serde(default)]
    pub dur: Option<f32>,
    /// M3 ke liye face event. Abhi parse hoga, use nahi hoga.
    #[serde(default)]
    pub face: Option<FaceEvent>,
    /// M3 ke liye camera event.
    #[serde(default)]
    pub camera: Option<CameraEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FaceEvent {
    pub expr: String,
    pub look: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CameraEvent {
    #[serde(rename = "move")]
    pub move_dir: String,
    pub amount: f32,
    pub dur: f32,
}

// ---------------- Skin manifest (§9 + user A) ----------------

#[derive(Debug, Clone, Deserialize)]
pub struct SkinManifest {
    pub skin: String,
    pub base_height: u32,
    pub parts: HashMap<String, Part>,
}

impl SkinManifest {
    /// Body part. §8 kehta hai body sabse upar hai; validate_structure()
    /// (skeleton.rs) ise hi root maanta hai.
    pub fn body(&self) -> Option<&Part> {
        self.parts.get("body")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Part {
    pub asset: String,
    pub pivot: Point,
    pub z: i32,
    /// body ke liye None; baaki har part ke liye Some (validate in skeleton.rs).
    #[serde(default)]
    pub parent: Option<String>,
    /// User A: parent ki image ke andar wo point jahan ye part ka pivot judta
    /// hai. body ke liye None. Body ke alawa har part ke liye required
    /// (skeleton.rs validate karta hai).
    #[serde(default)]
    pub attach: Option<Point>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JOB: &str = r#"{
      "job_id": "j_001",
      "video": { "w": 720, "h": 1280, "fps": 15, "duration_sec": 20, "piece_sec": 5 },
      "background": { "asset": "bg/park_day", "w": 2160, "h": 1280 },
      "weather": { "type": "sunny", "overlay_asset": "fx/sun_rays" },
      "delivery": { "mode": "url", "url": "https://example/x", "auth": "abc" },
      "characters": [
        { "id": "c1", "skin": "oggy", "height": 1.0, "x": 300, "y": 1000 },
        { "id": "c2", "skin": "jerry", "height": 0.6, "x": 500, "y": 1000 }
      ],
      "timeline": [
        { "t": 0.0, "char": "c1", "action": "idle" },
        { "t": 2.0, "char": "c1", "action": "walk", "to": { "x": 600, "y": 1000 }, "dur": 3.0 },
        { "t": 5.0, "char": "c1", "face": { "expr": "happy", "look": "right" } },
        { "t": 8.0, "camera": { "move": "E", "amount": 200, "dur": 2.0 } }
      ]
    }"#;

    const SAMPLE_SKIN: &str = r#"{
      "skin": "oggy",
      "base_height": 400,
      "parts": {
        "body": { "asset": "oggy/body", "pivot": { "x": 60, "y": 100 }, "z": 3 },
        "p10":  { "asset": "oggy/part_10", "pivot": { "x": 5,  "y": 5 }, "z": 4,
                  "parent": "body", "attach": { "x": 40, "y": 20 } },
        "p6":   { "asset": "oggy/part_6",  "pivot": { "x": 12, "y": 8 }, "z": 4,
                  "parent": "p10",  "attach": { "x": 5,  "y": 10 } }
      }
    }"#;

    #[test]
    fn parse_m1_sample_without_timeline() {
        let json = r#"{
          "job_id": "j_m1",
          "background": { "asset": "bg/x", "w": 720, "h": 1280 },
          "characters": []
        }"#;
        let job: Job = serde_json::from_str(json).unwrap();
        assert!(job.timeline.is_empty(), "timeline should default to empty");
    }

    #[test]
    fn parse_m2_timeline_actions() {
        let job: Job = serde_json::from_str(SAMPLE_JOB).unwrap();
        assert_eq!(job.timeline.len(), 4);

        // Event 0: Idle
        let ev0 = &job.timeline[0];
        assert_eq!(ev0.t, 0.0);
        assert_eq!(ev0.char.as_deref(), Some("c1"));
        assert_eq!(ev0.action.as_deref(), Some("idle"));
        assert!(ev0.to.is_none());

        // Event 1: Walk
        let ev1 = &job.timeline[1];
        assert_eq!(ev1.t, 2.0);
        assert_eq!(ev1.action.as_deref(), Some("walk"));
        let to = ev1.to.unwrap();
        assert_eq!(to.x, 600.0);
        assert_eq!(to.y, 1000.0);
        assert_eq!(ev1.dur.unwrap(), 3.0);

        // Event 2: Face (M3 ke liye, par parse ho raha hai)
        let ev2 = &job.timeline[2];
        let face = ev2.face.as_ref().unwrap();
        assert_eq!(face.expr, "happy");
        assert_eq!(face.look.as_deref(), Some("right"));

        // Event 3: Camera (M3 ke liye)
        let ev3 = &job.timeline[3];
        let cam = ev3.camera.as_ref().unwrap();
        assert_eq!(cam.move_dir, "E");
        assert_eq!(cam.amount, 200.0);
        assert_eq!(cam.dur, 2.0);
        assert!(ev3.char.is_none(), "camera event has no char");
    }

    #[test]
    fn parse_section_9_skin_with_attach() {
        let m: SkinManifest = serde_json::from_str(SAMPLE_SKIN).unwrap();
        assert_eq!(m.skin, "oggy");
        assert_eq!(m.base_height, 400);
        assert_eq!(m.parts.len(), 3);

        let body = m.body().expect("body must be present");
        assert_eq!(body.asset, "oggy/body");
        assert_eq!(body.pivot, Point::new(60.0, 100.0));
        assert_eq!(body.z, 3);
        assert!(body.parent.is_none(), "body cannot have a parent");
        assert!(body.attach.is_none(), "body cannot have an attach");

        let p10 = &m.parts["p10"];
        assert_eq!(p10.pivot, Point::new(5.0, 5.0));
        assert_eq!(p10.parent.as_deref(), Some("body"));
        assert_eq!(p10.attach, Some(Point::new(40.0, 20.0)));
    }

    #[test]
    fn float_fields_accept_integers() {
        let json = r#"{ "x": 60, "y": 100 }"#;
        let p: Point = serde_json::from_str(json).unwrap();
        assert_eq!(p.x, 60.0);
        assert_eq!(p.y, 100.0);
    }
    }
