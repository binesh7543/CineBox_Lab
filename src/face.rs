//! Face layer resolution (§8, §9).
//!
//! §8: "Face (alag layer, head ke upar): aankhein … aur mooh …"
//! §9: naam pattern — `skin/face_eyes_happy`, `skin/face_mouth_open`.
//!
//! **M1 interpretation (documented, M2 mein badal sakta):**
//!   - Eyes asset    = `{skin}/face_eyes_{expr}`              (ek per expr)
//!   - Mouth asset   = `{skin}/face_mouth_{mouth_state(expr)}` (happy->open, …)
//!   - `look`        = carried through, lekin **asset name mein nahi ghusata**
//!                     M1 mein. M2 mein direction-aware eye variants
//!                     (`face_eyes_{expr}_{look}`) yahi function return karega —
//!                     signature nahi badlega.
//!   - Default state = { expr: "happy", look: "center" } — §9 examples se.
//!
//! **M1 scope:** ye file sirf **naam** deti hai. Position skin.json ke rig se
//! aati hai (`face_eyes` / `face_mouth` keys ke saath, parent = head). Assets
//! load karna / place karna caller (main.rs) ka kaam.

pub const DEFAULT_EXPR: &str = "happy";
pub const DEFAULT_LOOK: &str = "center";

#[derive(Debug, Clone, PartialEq)]
pub struct FaceState {
    pub expr: String,
    pub look: String,
}

impl Default for FaceState {
    fn default() -> Self {
        Self {
            expr: DEFAULT_EXPR.to_string(),
            look: DEFAULT_LOOK.to_string(),
        }
    }
}

impl FaceState {
    pub fn new(expr: impl Into<String>, look: impl Into<String>) -> Self {
        Self { expr: expr.into(), look: look.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceAssets {
    pub eyes: String,
    pub mouth: String,
}

/// Full face resolution: skin name + state -> asset names.
pub fn resolve_face(skin: &str, state: &FaceState) -> FaceAssets {
    FaceAssets {
        eyes: eyes_asset(skin, &state.expr),
        mouth: mouth_asset(skin, &state.expr),
    }
}

/// `{skin}/face_eyes_{expr}` — M1 mein `look` include nahi hota.
pub fn eyes_asset(skin: &str, expr: &str) -> String {
    format!("{skin}/face_eyes_{expr}")
}

/// `{skin}/face_mouth_{state}` — state `mouth_state_for(expr)` se aata hai.
pub fn mouth_asset(skin: &str, expr: &str) -> String {
    format!("{skin}/face_mouth_{}", mouth_state_for(expr))
}

/// §8 kehta hai "mooh se expression badlega" — exprs ko mouth states mein map
/// karta hai. Naye expr tabhi jodo jab doc mein aaye (M2 planning).
pub fn mouth_state_for(expr: &str) -> &'static str {
    match expr {
        "happy" | "surprised" | "excited" => "open",
        "sad" | "angry" | "tired" => "closed",
        _ => "neutral",
    }
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_happy_center() {
        let s = FaceState::default();
        assert_eq!(s.expr, "happy");
        assert_eq!(s.look, "center");
    }

    #[test]
    fn default_assets_match_section_9_examples() {
        // §9 ne literally likha: "skin/face_eyes_happy", "skin/face_mouth_open".
        let a = resolve_face("oggy", &FaceState::default());
        assert_eq!(a.eyes, "oggy/face_eyes_happy");
        assert_eq!(a.mouth, "oggy/face_mouth_open");
    }

    #[test]
    fn custom_skin_prefix() {
        let a = resolve_face("jerry", &FaceState::default());
        assert_eq!(a.eyes, "jerry/face_eyes_happy");
        assert_eq!(a.mouth, "jerry/face_mouth_open");
    }

    #[test]
    fn sad_expr_selects_closed_mouth() {
        let s = FaceState::new("sad", "center");
        let a = resolve_face("oggy", &s);
        assert_eq!(a.eyes, "oggy/face_eyes_sad");
        assert_eq!(a.mouth, "oggy/face_mouth_closed");
    }

    #[test]
    fn unknown_expr_falls_back_to_neutral_mouth() {
        assert_eq!(mouth_state_for("confused"), "neutral");
        let s = FaceState::new("confused", "center");
        let a = resolve_face("oggy", &s);
        assert_eq!(a.mouth, "oggy/face_mouth_neutral");
        // Eyes still use expr as-is — asset missing ho to caller (assets.rs)
        // ka NotFound aayega, jo correct behaviour hai (skin author ki galti).
        assert_eq!(a.eyes, "oggy/face_eyes_confused");
    }

    #[test]
    fn look_does_not_affect_asset_names_in_m1() {
        // M1 contract: look carried through but not applied.
        // M2 jab `face_eyes_{expr}_{look}` add karega, ye test todna padega
        // — that's the point: interface change visible rahe.
        let a = resolve_face("oggy", &FaceState::new("happy", "right"));
        let b = resolve_face("oggy", &FaceState::new("happy", "left"));
        assert_eq!(a, b);
    }

    #[test]
    fn mouth_state_mapping_pinned() {
        // Explicit table so M2 changes are conscious.
        let cases = [
            ("happy", "open"),
            ("surprised", "open"),
            ("excited", "open"),
            ("sad", "closed"),
            ("angry", "closed"),
            ("tired", "closed"),
            ("neutral", "neutral"),
            ("", "neutral"),
            ("weird_expr", "neutral"),
        ];
        for (expr, want) in cases {
            assert_eq!(mouth_state_for(expr), want, "expr={expr}");
        }
    }

    #[test]
    fn eyes_asset_uses_expr_verbatim() {
        assert_eq!(eyes_asset("s", "happy"), "s/face_eyes_happy");
        assert_eq!(eyes_asset("s", "X_1"), "s/face_eyes_X_1");
    }

    #[test]
    fn mouth_asset_routes_through_state_mapping() {
        assert_eq!(mouth_asset("s", "happy"), "s/face_mouth_open");
        assert_eq!(mouth_asset("s", "sad"), "s/face_mouth_closed");
        assert_eq!(mouth_asset("s", "?"), "s/face_mouth_neutral");
    }
          }
