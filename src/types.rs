//! Job JSON (§7) aur skin manifest (§9) ke data types.
//!
//! M1 scope: sirf wo fields jo ek frame render karne ke liye chahiye.
//! §7 ke baaki sections (weather, delivery, timeline) is file mein nahi hain —
//! serde_json unhe silently ignore karta hai (deny_unknown_fields jaan-boojh ke nahi
//! lagaya). Wo M2–M5 mein jodenge.
//!
//! Assumption (user B): character.x / character.y = body part ke pivot ki screen
//! position. Zameen wala anchor (pair ke neeche) M2 mein dekhenge.

use serde::Deserialize;
use std::collections::HashMap;

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
        { "t": 2.0, "char": "c1", "action": "walk", "to": { "x": 600, "y": 1000 }, "dur": 3.0 }
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
    fn parse_section_7_sample() {
        let job: Job = serde_json::from_str(SAMPLE_JOB).unwrap();
        assert_eq!(job.job_id, "j_001");
        assert_eq!(job.video.w, Some(720));
        assert_eq!(job.video.h, Some(1280));
        assert_eq!(job.video.fps, Some(15));
        assert_eq!(job.video.duration_sec, Some(20.0));
        assert_eq!(job.video.piece_sec, Some(5.0));
        assert_eq!(job.background.asset, "bg/park_day");
        assert_eq!(job.background.w, 2160);
        assert_eq!(job.background.h, 1280);
        assert_eq!(job.characters.len(), 2);
        assert_eq!(job.characters[0].id, "c1");
        assert_eq!(job.characters[0].skin, "oggy");
        assert_eq!(job.characters[0].height, 1.0);
        assert_eq!(job.characters[0].x, 300);
        assert_eq!(job.characters[0].y, 1000);
        assert_eq!(job.characters[1].height, 0.6);
    }

    #[test]
    fn job_unknown_fields_ignored() {
        // weather / delivery / timeline JSON mein hain, Job struct mein nahi.
        // serde_json default behaviour: ignore karo, error mat do.
        let _: Job = serde_json::from_str(SAMPLE_JOB).expect("unknown fields must be ignored");
    }

    #[test]
    fn job_missing_video_uses_empty_opts() {
        let json = r#"{
          "job_id": "j_no_video",
          "background": { "asset": "bg/x", "w": 720, "h": 1280 },
          "characters": []
        }"#;
        let job: Job = serde_json::from_str(json).unwrap();
        assert!(job.video.w.is_none());
        assert!(job.video.fps.is_none());
    }

    #[test]
    fn character_height_defaults_to_one() {
        let json = r#"{
          "job_id": "j",
          "background": { "asset": "bg/x", "w": 720, "h": 1280 },
          "characters": [ { "id": "c1", "skin": "s", "x": 0, "y": 0 } ]
        }"#;
        let job: Job = serde_json::from_str(json).unwrap();
        assert_eq!(job.characters[0].height, 1.0);
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

        let p6 = &m.parts["p6"];
        assert_eq!(p6.parent.as_deref(), Some("p10"));
        assert_eq!(p6.attach, Some(Point::new(5.0, 10.0)));
    }

    #[test]
    fn body_attach_defaults_to_none() {
        // Skin jismein body pe attach key hi nahi — parse fail nahi hona chahiye.
        let json = r#"{
          "skin": "s", "base_height": 400,
          "parts": {
            "body": { "asset": "b", "pivot": { "x": 0, "y": 0 }, "z": 0 }
          }
        }"#;
        let m: SkinManifest = serde_json::from_str(json).unwrap();
        assert!(m.body().unwrap().attach.is_none());
        assert!(m.body().unwrap().parent.is_none());
    }

    #[test]
    fn skin_missing_attach_still_parses() {
        // attach field na ho to parse succeed (skeleton.rs validate_structure use karega).
        let json = r#"{
          "skin": "s", "base_height": 400,
          "parts": {
            "body": { "asset": "b", "pivot": { "x": 0, "y": 0 }, "z": 0 },
            "p3":   { "asset": "l", "pivot": { "x": 0, "y": 0 }, "z": 1, "parent": "body" }
          }
        }"#;
        let m: SkinManifest = serde_json::from_str(json).unwrap();
        assert!(m.parts["p3"].attach.is_none());
    }

    #[test]
    fn float_fields_accept_integers() {
        // JSON mein { "x": 60 } -> Point { x: 60.0 }. serde_json coerces int->f32.
        let json = r#"{ "x": 60, "y": 100 }"#;
        let p: Point = serde_json::from_str(json).unwrap();
        assert_eq!(p.x, 60.0);
        assert_eq!(p.y, 100.0);
    }
  }
