//! Skeleton: skin manifest + character position -> world-space part transforms (§8).
//!
//! M2 Update: 
//!   - Rotation support added (parent chain accumulation).
//!   - Movement library (idle, walk, wave) keyframes.
//!   - `ResolvedPart` now has `angle_rad`.

use crate::assets::CachedImage;
use crate::timeline::ActionState;
use crate::types::SkinManifest;
use std::collections::HashMap;
use std::sync::Arc;

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
            SkeletonError::MissingParentField { part } => write!(f, "part '{part}' is not body but has no parent"),
            SkeletonError::MissingParent { part, parent } => write!(f, "part '{part}' references unknown parent '{parent}'"),
            SkeletonError::MissingAttach { part } => write!(f, "part '{part}' is not body but has no attach"),
            SkeletonError::Cycle { part } => write!(f, "part chain does not reach body (cycle near '{part}')"),
            SkeletonError::MissingImage { asset } => write!(f, "image for asset '{asset}' not pre-loaded"),
        }
    }
}

impl std::error::Error for SkeletonError {}

#[derive(Debug, Clone)]
pub struct ResolvedPart {
    pub name: String,
    pub z: i32,
    pub image: Arc<CachedImage>,
    /// World-space position of the part image's top-left (pre-rotation).
    pub top_left: (f32, f32),
    /// World-space position of the pivot.
    pub world_pivot: (f32, f32),
    /// M2: Rotation angle in radians (accumulated from parent).
    pub angle_rad: f32,
}

#[derive(Debug, Clone)]
pub struct ResolvedPose {
    pub parts: Vec<ResolvedPart>,
}

impl ResolvedPose {
    pub fn get(&self, name: &str) -> Option<&ResolvedPart> {
        self.parts.iter().find(|p| p.name == name)
    }
}

pub fn validate_structure(m: &SkinManifest) -> Result<(), SkeletonError> {
    let body = m.body().ok_or(SkeletonError::MissingBody)?;
    if body.parent.is_some() {
        return Err(SkeletonError::BodyHasParent);
    }
    if body.attach.is_some() {
        return Err(SkeletonError::BodyHasAttach);
    }

    for (name, part) in &m.parts {
        if name == "body" { continue; }
        let parent = part.parent.as_ref().ok_or_else(|| SkeletonError::MissingParentField { part: name.clone() })?;
        if !m.parts.contains_key(parent) {
            return Err(SkeletonError::MissingParent { part: name.clone(), parent: parent.clone() });
        }
        if part.attach.is_none() {
            return Err(SkeletonError::MissingAttach { part: name.clone() });
        }
    }

    for name in m.parts.keys() {
        let mut cur = name.as_str();
        let mut visited = std::collections::HashSet::new();
        while cur != "body" {
            if !visited.insert(cur) {
                return Err(SkeletonError::Cycle { part: name.clone() });
            }
            cur = m.parts[cur].parent.as_ref().unwrap().as_str();
        }
    }
    Ok(())
}

/// M1 compatibility wrapper (calls animated pose with idle state).
pub fn resolve_pose(
    manifest: &SkinManifest,
    char_pos: (i32, i32),
    images: &HashMap<String, Arc<CachedImage>>,
) -> Result<ResolvedPose, SkeletonError> {
    resolve_animated_pose(manifest, (char_pos.0 as f32, char_pos.1 as f32), images, ActionState::Idle, 0.0, 0.0)
}

/// M2: Resolve pose with action, walk_phase and time_in_action.
pub fn resolve_animated_pose(
    manifest: &SkinManifest,
    char_pos: (f32, f32),
    images: &HashMap<String, Arc<CachedImage>>,
    action: ActionState,
    walk_phase: f32,
    action_time: f32,
) -> Result<ResolvedPose, SkeletonError> {
    validate_structure(manifest)?;

    struct T {
        world_pivot: (f32, f32),
        top_left: (f32, f32),
        world_angle: f32,
        image: Arc<CachedImage>,
    }
    let mut transforms: HashMap<String, T> = HashMap::new();

    let body = manifest.body().expect("validated");
    let body_img = images.get(&body.asset).ok_or_else(|| SkeletonError::MissingImage { asset: body.asset.clone() })?.clone();
    let body_ps = body_img.pivot_scale();
    let body_wp = char_pos;
    let body_angle = 0.0; // Body rotation will be handled in M3 if needed
    let body_tl = (
        body_wp.0 - body.pivot.x * body_ps.0,
        body_wp.1 - body.pivot.y * body_ps.1,
    );

    transforms.insert("body".to_string(), T { world_pivot: body_wp, top_left: body_tl, world_angle: body_angle, image: body_img });

    loop {
        let pending: Vec<String> = manifest.parts.keys().filter(|k| !transforms.contains_key(*k)).cloned().collect();
        if pending.is_empty() { break; }
        let mut progress = false;
        for name in &pending {
            let part = &manifest.parts[name];
            let parent_name = part.parent.as_ref().expect("validated");
            
            let (pw, parent_pivot, parent_pscale, p_angle) = match transforms.get(parent_name) {
                Some(t) => (
                    t.world_pivot,
                    manifest.parts.get(parent_name).expect("validated").pivot,
                    t.image.pivot_scale(),
                    t.world_angle,
                ),
                None => continue,
            };

            let image = match images.get(&part.asset) {
                Some(i) => i.clone(),
                None => return Err(SkeletonError::MissingImage { asset: part.asset.clone() }),
            };
            let attach = part.attach.expect("validated");

            // M2: Calculate local angle and accumulate world angle
            let local_angle = get_local_angle(name, action, walk_phase, action_time);
            let world_angle = p_angle + local_angle;

            // M2: Rotate attachment vector by parent's world angle
            let dx = (attach.x - parent_pivot.x) * parent_pscale.0;
            let dy = (attach.y - parent_pivot.y) * parent_pscale.1;
            
            let cos_a = p_angle.cos();
            let sin_a = p_angle.sin();
            let rx = dx * cos_a - dy * sin_a;
            let ry = dx * sin_a + dy * cos_a;

            let world_pivot = (pw.0 + rx, pw.1 + ry);

            let my_ps = image.pivot_scale();
            let top_left = (
                world_pivot.0 - part.pivot.x * my_ps.0,
                world_pivot.1 - part.pivot.y * my_ps.1,
            );

            transforms.insert(name.clone(), T { world_pivot, top_left, world_angle, image });
            progress = true;
        }
        if !progress {
            let stuck = pending.into_iter().next().unwrap();
            return Err(SkeletonError::Cycle { part: stuck });
        }
    }

    let mut parts: Vec<ResolvedPart> = manifest.parts.iter().map(|(name, part)| {
        let t = transforms.get(name).expect("all parts resolved");
        ResolvedPart {
            name: name.clone(),
            z: part.z,
            image: t.image.clone(),
            top_left: t.top_left,
            world_pivot: t.world_pivot,
            angle_rad: t.world_angle,
        }
    }).collect();
    parts.sort_by(|a, b| a.z.cmp(&b.z).then_with(|| a.name.cmp(&b.name)));
    Ok(ResolvedPose { parts })
}

/// M2 Movement Library: returns local angle in radians.
fn get_local_angle(part_name: &str, action: ActionState, phase: f32, time: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    match action {
        ActionState::Idle => 0.0,
        ActionState::Walk => {
            let cycle = phase * TAU;
            match part_name {
                "p3" => cycle.sin() * 0.5,                      // Right leg
                "p4" => (cycle + PI).sin() * 0.5,               // Left leg
                "p10" => (cycle + PI).sin() * 0.4,              // Right arm
                "p8" => cycle.sin() * 0.4,                      // Left arm
                _ => 0.0,
            }
        }
        ActionState::Wave => match part_name {
            "p10" => -1.5,                                      // Raise shoulder
            "p6" => (time * 10.0).sin() * 0.5,                  // Wave arm
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> SkinManifest {
        serde_json::from_str(json).expect("valid skin json")
    }

    fn img(src: (u32, u32), disp: (u32, u32)) -> Arc<CachedImage> {
        Arc::new(CachedImage { rgba: vec![0u8; (disp.0 * disp.1 * 4) as usize], width: disp.0, height: disp.1, src_width: src.0, src_height: src.1 })
    }

    fn assert_close(actual: (f32, f32), expected: (f32, f32)) {
        let eps = 1e-4;
        assert!((actual.0 - expected.0).abs() < eps && (actual.1 - expected.1).abs() < eps, "expected {:?}, got {:?}", expected, actual);
    }

    const VALID: &str = r#"{
      "skin": "t", "base_height": 100,
      "parts": {
        "body": { "asset": "b",  "pivot": {"x": 50, "y": 50}, "z": 3 },
        "p10":  { "asset": "s",  "pivot": {"x": 10, "y": 10}, "z": 4,
                  "parent": "body", "attach": {"x": 60, "y": 70} }
      }
    }"#;

    #[test]
    fn resolve_simple_rig_scale_one() {
        let m = manifest(VALID);
        let mut images = HashMap::new();
        images.insert("b".to_string(), img((100, 100), (100, 100)));
        images.insert("s".to_string(), img((20, 20), (20, 20)));

        let pose = resolve_pose(&m, (300, 400), &images).unwrap();
        assert_close(pose.get("body").unwrap().top_left, (250.0, 350.0));
        let p10 = pose.get("p10").unwrap();
        assert_close(p10.world_pivot, (310.0, 420.0));
        assert_close(p10.top_left, (300.0, 410.0));
        assert_eq!(p10.angle_rad, 0.0); // Idle, so no rotation
    }

    #[test]
    fn resolve_pose_rotates_parts() {
        let m = manifest(VALID);
        let mut images = HashMap::new();
        images.insert("b".to_string(), img((100, 100), (100, 100)));
        images.insert("s".to_string(), img((20, 20), (20, 20)));

        // Walk state, phase 0.25 (quarter cycle). p10 is an arm, so it should rotate.
        let pose = resolve_animated_pose(&m, (300.0, 400.0), &images, ActionState::Walk, 0.25, 0.0).unwrap();
        let p10 = pose.get("p10").unwrap();
        
        assert!(p10.angle_rad != 0.0, "Walk state should rotate arm");
    }

    #[test]
    fn resolve_pose_rotates_attachment() {
        let m = manifest(r#"{
          "skin":"t","base_height":100,
          "parts": {
            "body": {"asset":"b","pivot":{"x":0,"y":0},"z":0},
            "p10":  {"asset":"a","pivot":{"x":0,"y":0},"z":1,
                     "parent":"body","attach":{"x":10,"y":0}},
            "p6":   {"asset":"c","pivot":{"x":0,"y":0},"z":2,
                     "parent":"p10","attach":{"x":10,"y":0}}
          }
        }"#);
        let mut images = HashMap::new();
        for a in ["b", "a", "c"] { images.insert(a.to_string(), img((10, 10), (10, 10))); }

        let pose = resolve_animated_pose(&m, (0.0, 0.0), &images, ActionState::Walk, 0.75, 0.0).unwrap();
        
        let p10 = pose.get("p10").unwrap();
        let p6 = pose.get("p6").unwrap();

        // p10 world_pivot should be rotated by ~0.4 rad
        assert!(p10.world_pivot.0 < 10.5); 
        assert!(p10.world_pivot.1 > 0.0);

        // p6 attach point should also be rotated by p10's angle
        assert!(p6.world_pivot.0 < 20.5);
        assert!(p6.world_pivot.1 > 0.0);
    }
                                                    }
        
