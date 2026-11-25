//! Skeleton: skin manifest + character position -> world-space part transforms (§8).
//!
//! **M1 assumption (user B):** `character.x, character.y` = **body part ke pivot**
//! ki screen position. Zameen wala anchor (pair ke neeche) M2 mein dekhenge.
//! Ye assumption `body_pivot_lands_at_character_xy_m1_assumption` test mein pin hai.
//!
//! **M1 scope:** default pose. Koi action, koi angle nahi. Movement library
//! M2 mein `resolve_pose_animated(...)` ke roop mein aayegi; ye struct tabhi
//! badlega jab zaroorat ho (`angle_rad` field etc.).
//!
//! **Rig (§8 + user A):**
//!   - body  = root (parent=None, attach=None)
//!   - every other part: parent = existing key, attach = parent ki **source**
//!     image ke andar wo point jahan is part ka pivot judta hai.
//!
//! **Math (angles = 0, M1):**
//!   world_pivot(body) = (char.x, char.y)                                    // user B
//!   world_pivot(P)    = world_pivot(Q)
//!                     + (attach(P) - pivot(Q)) * pivot_scale(Q)             // user A
//!   top_left(P)       = world_pivot(P) - pivot(P) * pivot_scale(P)
//!
//! Output z-ascending (painter's algorithm: chhoti z pehle draw hoti hai).

use crate::assets::CachedImage;
use crate::types::SkinManifest;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------- Errors ----------------

#[derive(Debug)]
pub enum SkeletonError {
    MissingBody,
    BodyHasParent,
    BodyHasAttach,
    MissingParentField { part: String },
    MissingParent { part: String, parent: String },
    MissingAttach { part: String },
    Cycle { part: String },
    MissingImage { asset: String },
}

impl std::fmt::Display for SkeletonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkeletonError::MissingBody => write!(f, "skin.json missing 'body' part"),
            SkeletonError::BodyHasParent => write!(f, "body must not have a parent"),
            SkeletonError::BodyHasAttach => write!(f, "body must not have an attach"),
            SkeletonError::MissingParentField { part } =>
                write!(f, "part '{part}' is not body but has no parent"),
            SkeletonError::MissingParent { part, parent } =>
                write!(f, "part '{part}' references unknown parent '{parent}'"),
            SkeletonError::MissingAttach { part } =>
                write!(f, "part '{part}' is not body but has no attach"),
            SkeletonError::Cycle { part } =>
                write!(f, "part chain does not reach body (cycle near '{part}')"),
            SkeletonError::MissingImage { asset } =>
                write!(f, "image for asset '{asset}' not pre-loaded"),
        }
    }
}

impl std::error::Error for SkeletonError {}

// ---------------- Output ----------------

#[derive(Debug, Clone)]
pub struct ResolvedPart {
    /// Manifest key ("body", "p10", "p6", …).
    pub name: String,
    pub z: i32,
    pub image: Arc<CachedImage>,
    /// World-space position of the part image's top-left (pre-rotation).
    pub top_left: (f32, f32),
    /// World-space position of the pivot. Equals `character.x,y` for body (user B).
    /// M2 rotation iske around hogi.
    pub world_pivot: (f32, f32),
}

#[derive(Debug, Clone)]
pub struct ResolvedPose {
    /// Sorted z-ascending, ties broken by name (deterministic).
    pub parts: Vec<ResolvedPart>,
}

impl ResolvedPose {
    pub fn get(&self, name: &str) -> Option<&ResolvedPart> {
        self.parts.iter().find(|p| p.name == name)
    }
}

// ---------------- Validation ----------------

/// Structural checks on a parsed skin manifest. Pure — koi I/O nahi.
/// `resolve_pose` internally bhi call karta hai; caller jab job shuru kare
/// tab "fail fast" ke liye bhi call kar sakta hai.
pub fn validate_structure(m: &SkinManifest) -> Result<(), SkeletonError> {
    let body = m.body().ok_or(SkeletonError::MissingBody)?;
    if body.parent.is_some() {
        return Err(SkeletonError::BodyHasParent);
    }
    if body.attach.is_some() {
        return Err(SkeletonError::BodyHasAttach);
    }

    // Parent field + existence + attach presence for non-body parts.
    for (name, part) in &m.parts {
        if name == "body" {
            continue;
        }
        let parent = part
            .parent
            .as_ref()
            .ok_or_else(|| SkeletonError::MissingParentField { part: name.clone() })?;
        if !m.parts.contains_key(parent) {
            return Err(SkeletonError::MissingParent {
                part: name.clone(),
                parent: parent.clone(),
            });
        }
        if part.attach.is_none() {
            return Err(SkeletonError::MissingAttach { part: name.clone() });
        }
    }

    // Cycle: walk each part up; must reach body in finite steps.
    for name in m.parts.keys() {
        let mut cur = name.as_str();
        let mut visited = std::collections::HashSet::new();
        while cur != "body" {
            if !visited.insert(cur) {
                return Err(SkeletonError::Cycle { part: name.clone() });
            }
            // parent exists (checked above); unwrap safe.
            cur = m.parts[cur].parent.as_ref().unwrap().as_str();
        }
    }

    Ok(())
}

// ---------------- Resolution ----------------

/// Skin manifest + character position + pre-loaded images -> world transforms.
///
/// **Caller contract:** `images` keyed by `part.asset` (§9 ka "asset" field).
/// Caller ne unhe `AssetLoader::load(name, character.height)` se load kiya hoga,
/// isliye `CachedImage::pivot_scale()` already correct scale return karta hai.
pub fn resolve_pose(
    manifest: &SkinManifest,
    char_pos: (i32, i32),
    images: &HashMap<String, Arc<CachedImage>>,
) -> Result<ResolvedPose, SkeletonError> {
    validate_structure(manifest)?;

    struct T {
        world_pivot: (f32, f32),
        top_left: (f32, f32),
        image: Arc<CachedImage>,
    }
    let mut transforms: HashMap<String, T> = HashMap::new();

    // Body first (root, no parent, user B: lands exactly on char_pos).
    let body = manifest.body().expect("validated");
    let body_img = images
        .get(&body.asset)
        .ok_or_else(|| SkeletonError::MissingImage { asset: body.asset.clone() })?
        .clone();
    let body_ps = body_img.pivot_scale();
    let body_wp = (char_pos.0 as f32, char_pos.1 as f32);
    let body_tl = (
        body_wp.0 - body.pivot.x * body_ps.0,
        body_wp.1 - body.pivot.y * body_ps.1,
    );
    transforms.insert(
        "body".to_string(),
        T { world_pivot: body_wp, top_left: body_tl, image: body_img },
    );

    // Iteratively resolve children whose parent is already resolved.
    // O(N²) worst case, N ~= 12 parts (§8). Negligible per frame.
    loop {
        let pending: Vec<String> = manifest
            .parts
            .keys()
            .filter(|k| !transforms.contains_key(*k))
            .cloned()
            .collect();
        if pending.is_empty() {
            break;
        }
        let mut progress = false;
        for name in &pending {
            let part = &manifest.parts[name];
            let parent_name = part.parent.as_ref().expect("validated");
            let (pw, parent_pivot, parent_pscale) = match transforms.get(parent_name) {
                Some(t) => (
                    t.world_pivot,
                    manifest.parts.get(parent_name).expect("validated").pivot,
                    t.image.pivot_scale(),
                ),
                None => continue,
            };
            let image = match images.get(&part.asset) {
                Some(i) => i.clone(),
                None => return Err(SkeletonError::MissingImage { asset: part.asset.clone() }),
            };
            let attach = part.attach.expect("validated");

            // Attach offset in parent's source image space, scaled to display.
            let local = (attach.x - parent_pivot.x, attach.y - parent_pivot.y);
            let disp = (local.0 * parent_pscale.0, local.1 * parent_pscale.1);
            let world_pivot = (pw.0 + disp.0, pw.1 + disp.1);

            let my_ps = image.pivot_scale();
            let top_left = (
                world_pivot.0 - part.pivot.x * my_ps.0,
                world_pivot.1 - part.pivot.y * my_ps.1,
            );

            transforms.insert(
                name.clone(),
                T { world_pivot, top_left, image },
            );
            progress = true;
        }
        if !progress {
            let stuck = pending.into_iter().next().unwrap();
            return Err(SkeletonError::Cycle { part: stuck });
        }
    }

    let mut parts: Vec<ResolvedPart> = manifest
        .parts
        .iter()
        .map(|(name, part)| {
            let t = transforms.get(name).expect("all parts resolved");
            ResolvedPart {
                name: name.clone(),
                z: part.z,
                image: t.image.clone(),
                top_left: t.top_left,
                world_pivot: t.world_pivot,
            }
        })
        .collect();
    parts.sort_by(|a, b| a.z.cmp(&b.z).then_with(|| a.name.cmp(&b.name)));
    Ok(ResolvedPose { parts })
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> SkinManifest {
        serde_json::from_str(json).expect("valid skin json")
    }

    fn img(src: (u32, u32), disp: (u32, u32)) -> Arc<CachedImage> {
        Arc::new(CachedImage {
            rgba: vec![0u8; (disp.0 * disp.1 * 4) as usize],
            width: disp.0,
            height: disp.1,
            src_width: src.0,
            src_height: src.1,
        })
    }

    fn assert_close(actual: (f32, f32), expected: (f32, f32)) {
        let eps = 1e-4;
        assert!(
            (actual.0 - expected.0).abs() < eps && (actual.1 - expected.1).abs() < eps,
            "expected {:?}, got {:?}",
            expected,
            actual
        );
    }

    // ---------------- validate_structure ----------------

    const VALID: &str = r#"{
      "skin": "t", "base_height": 100,
      "parts": {
        "body": { "asset": "b",  "pivot": {"x": 50, "y": 50}, "z": 3 },
        "p10":  { "asset": "s",  "pivot": {"x": 10, "y": 10}, "z": 4,
                  "parent": "body", "attach": {"x": 60, "y": 70} }
      }
    }"#;

    #[test]
    fn validate_ok_on_valid_manifest() {
        validate_structure(&manifest(VALID)).unwrap();
    }

    #[test]
    fn validate_missing_body() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": { "p1": {"asset":"a","pivot":{"x":0,"y":0},"z":1,
                            "parent":"body","attach":{"x":0,"y":0}} }
        }"#);
        assert!(matches!(validate_structure(&m), Err(SkeletonError::MissingBody)));
    }

    #[test]
    fn validate_body_has_parent() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": { "body": {"asset":"b","pivot":{"x":0,"y":0},"z":0,
                              "parent":"p1","attach":{"x":0,"y":0}} }
        }"#);
        assert!(matches!(validate_structure(&m), Err(SkeletonError::BodyHasParent)));
    }

    #[test]
    fn validate_body_has_attach() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": { "body": {"asset":"b","pivot":{"x":0,"y":0},"z":0,
                              "attach":{"x":0,"y":0}} }
        }"#);
        assert!(matches!(validate_structure(&m), Err(SkeletonError::BodyHasAttach)));
    }

    #[test]
    fn validate_non_body_missing_parent_field() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": {
            "body": {"asset":"b","pivot":{"x":0,"y":0},"z":0},
            "p1":   {"asset":"a","pivot":{"x":0,"y":0},"z":1,"attach":{"x":0,"y":0}}
          }
        }"#);
        assert!(matches!(validate_structure(&m), Err(SkeletonError::MissingParentField { .. })));
    }

    #[test]
    fn validate_non_body_missing_attach() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": {
            "body": {"asset":"b","pivot":{"x":0,"y":0},"z":0},
            "p1":   {"asset":"a","pivot":{"x":0,"y":0},"z":1,"parent":"body"}
          }
        }"#);
        assert!(matches!(validate_structure(&m), Err(SkeletonError::MissingAttach { .. })));
    }

    #[test]
    fn validate_unknown_parent() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": {
            "body": {"asset":"b","pivot":{"x":0,"y":0},"z":0},
            "p1":   {"asset":"a","pivot":{"x":0,"y":0},"z":1,
                     "parent":"nope","attach":{"x":0,"y":0}}
          }
        }"#);
        match validate_structure(&m) {
            Err(SkeletonError::MissingParent { part, parent }) => {
                assert_eq!(part, "p1");
                assert_eq!(parent, "nope");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn validate_cycle_detected() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": {
            "body": {"asset":"b","pivot":{"x":0,"y":0},"z":0},
            "p1":   {"asset":"a","pivot":{"x":0,"y":0},"z":1,
                     "parent":"p2","attach":{"x":0,"y":0}},
            "p2":   {"asset":"c","pivot":{"x":0,"y":0},"z":2,
                     "parent":"p1","attach":{"x":0,"y":0}}
          }
        }"#);
        assert!(matches!(validate_structure(&m), Err(SkeletonError::Cycle { .. })));
    }

    // ---------------- resolve_pose ----------------

    #[test]
    fn body_pivot_lands_at_character_xy_m1_assumption() {
        // User B: character.x, y = body.pivot ki screen position.
        // (Zameen anchor M2 mein aayega — tab ye test bhi evolve hoga.)
        let m = manifest(VALID);
        let mut images = HashMap::new();
        images.insert("b".to_string(), img((100, 100), (100, 100)));
        images.insert("s".to_string(), img((20, 20), (20, 20)));

        let pose = resolve_pose(&m, (300, 400), &images).unwrap();
        let body = pose.get("body").expect("body present");
        assert_close(body.world_pivot, (300.0, 400.0));
    }

    #[test]
    fn resolve_simple_rig_scale_one() {
        // body: 100x100 src/disp, pivot (50,50)
        // child: 20x20 src/disp, pivot (10,10), attach (60,70) in body source
        // char_pos = (300, 400)
        let m = manifest(VALID);
        let mut images = HashMap::new();
        images.insert("b".to_string(), img((100, 100), (100, 100)));
        images.insert("s".to_string(), img((20, 20), (20, 20)));

        let pose = resolve_pose(&m, (300, 400), &images).unwrap();

        let body = pose.get("body").unwrap();
        assert_close(body.top_left, (250.0, 350.0));

        // attach offset: (60-50, 70-50) * 1.0 = (10, 20)
        // child.world_pivot = (300+10, 400+20) = (310, 420)
        // child.top_left     = (310-10, 420-10) = (300, 410)
        let p10 = pose.get("p10").unwrap();
        assert_close(p10.world_pivot, (310.0, 420.0));
        assert_close(p10.top_left, (300.0, 410.0));
    }

    #[test]
    fn resolve_simple_rig_scale_half() {
        // Same rig, but pre-loaded at character.height=0.5 (all images scaled 0.5).
        let m = manifest(VALID);
        let mut images = HashMap::new();
        images.insert("b".to_string(), img((100, 100), (50, 50)));
        images.insert("s".to_string(), img((20, 20), (10, 10)));

        let pose = resolve_pose(&m, (300, 400), &images).unwrap();

        // body pivot (50,50)*0.5=(25,25); wp=(300,400); tl=(275,375)
        assert_close(pose.get("body").unwrap().top_left, (275.0, 375.0));

        // attach offset: (10,20)*0.5=(5,10); wp=(305,410)
        // child pivot (10,10)*0.5=(5,5); tl=(300,405)
        let p10 = pose.get("p10").unwrap();
        assert_close(p10.world_pivot, (305.0, 410.0));
        assert_close(p10.top_left, (300.0, 405.0));
    }

    #[test]
    fn resolve_three_part_chain() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": {
            "body": {"asset":"b","pivot":{"x":50,"y":50},"z":3},
            "p10":  {"asset":"a","pivot":{"x":10,"y":10},"z":4,
                     "parent":"body","attach":{"x":60,"y":70}},
            "p6":   {"asset":"c","pivot":{"x":5,"y":5},"z":5,
                     "parent":"p10","attach":{"x":15,"y":15}}
          }
        }"#);
        let mut images = HashMap::new();
        for a in ["b", "a", "c"] {
            images.insert(a.to_string(), img((100, 100), (100, 100)));
        }

        let pose = resolve_pose(&m, (300, 400), &images).unwrap();

        // body.wp=(300,400), body.tl=(250,350)
        assert_close(pose.get("body").unwrap().world_pivot, (300.0, 400.0));
        // p10: attach-pivot=(10,20); p10.wp=(310,420); p10.tl=(300,410)
        let p10 = pose.get("p10").unwrap();
        assert_close(p10.world_pivot, (310.0, 420.0));
        assert_close(p10.top_left, (300.0, 410.0));
        // p6: attach-p10.pivot=(5,5); p6.wp=(315,425); p6.tl=(310,420)
        let p6 = pose.get("p6").unwrap();
        assert_close(p6.world_pivot, (315.0, 425.0));
        assert_close(p6.top_left, (310.0, 420.0));
    }

    #[test]
    fn parts_sorted_by_z_then_name() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": {
            "body": {"asset":"b","pivot":{"x":0,"y":0},"z":3},
            "p10":  {"asset":"a","pivot":{"x":0,"y":0},"z":4,
                     "parent":"body","attach":{"x":0,"y":0}},
            "p8":   {"asset":"c","pivot":{"x":0,"y":0},"z":4,
                     "parent":"body","attach":{"x":0,"y":0}}
          }
        }"#);
        let mut images = HashMap::new();
        for a in ["b", "a", "c"] {
            images.insert(a.to_string(), img((10, 10), (10, 10)));
        }
        let pose = resolve_pose(&m, (0, 0), &images).unwrap();
        let names: Vec<&str> = pose.parts.iter().map(|p| p.name.as_str()).collect();
        // FIX: String comparison mein "p10" "p8" se pehle aata hai kyunki '1' < '8'.
        // Isliye alphabetical sorting ka result ["body", "p10", "p8"] hona chahiye.
        assert_eq!(names, vec!["body", "p10", "p8"]);
    }

    #[test]
    fn missing_image_errors() {
        let m = manifest(VALID);
        let mut images = HashMap::new();
        images.insert("b".to_string(), img((100, 100), (100, 100)));
        // "s" missing.

        match resolve_pose(&m, (0, 0), &images) {
            Err(SkeletonError::MissingImage { asset }) => assert_eq!(asset, "s"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn all_parts_present_in_output() {
        let m = manifest(VALID);
        let mut images = HashMap::new();
        images.insert("b".to_string(), img((10, 10), (10, 10)));
        images.insert("s".to_string(), img((10, 10), (10, 10)));
        let pose = resolve_pose(&m, (0, 0), &images).unwrap();
        assert_eq!(pose.parts.len(), m.parts.len());
    }
        }
