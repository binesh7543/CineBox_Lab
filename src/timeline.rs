//! Timeline state tracking for characters (§7, §12).
//!
//! M2 scope:
//!   - Tracks action state ("idle", "walk", "wave").
//!   - Interpolates `x, y` position over time for walk/run.
//!   - Interpolates walk cycle phase (0.0 to 1.0) for leg/arm angles.
//!
//! M3 Fix (Bug 1): Forward fold implement kiya gaya hai taaki position 
//! events ke beech mein hold rahe, reset na ho.

use crate::types::{Job, Point, TimelineEvent, WALK_CYCLE_PERIOD_SEC};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionState {
    Idle,
    Walk,
    Wave,
}

#[derive(Debug, Clone, Copy)]
pub struct CharacterState {
    pub action: ActionState,
    /// Current interpolated position.
    pub pos: (f32, f32),
    /// Walk cycle phase (0.0 to 1.0). Only relevant for Walk.
    pub walk_phase: f32,
    /// Time spent in current action (seconds).
    pub action_elapsed: f32,
}

impl CharacterState {
    /// Default state M1 se match karta hai (idle, base pos).
    pub fn initial(pos: (i32, i32)) -> Self {
        Self {
            action: ActionState::Idle,
            pos: (pos.0 as f32, pos.1 as f32),
            walk_phase: 0.0,
            action_elapsed: 0.0,
        }
    }
}

/// Character ka state calculate karo us specific time `t` par.
pub fn get_character_state(
    job: &Job,
    char_id: &str,
    initial_pos: (i32, i32),
    t: f32,
) -> CharacterState {
    // 1. Us character ke saare events nikaalo jo `t` se pehle ya barabar hain.
    let mut relevant_events: Vec<&TimelineEvent> = job
        .timeline
        .iter()
        .filter(|ev| ev.char.as_deref() == Some(char_id) && ev.t <= t)
        .collect();

    // Sort by time
    relevant_events.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

    let mut state = CharacterState::initial(initial_pos);

    // 2. Forward fold: Position ko events ke through accumulate karo (Bug 1 Fix)
    for ev in &relevant_events {
        if let Some(to) = ev.to {
            let event_duration = ev.dur.unwrap_or(1.0);
            
            if t >= ev.t + event_duration {
                // Event poora khatam ho chuka hai, position destination pe lock karo
                state.pos = (to.x, to.y);
            } else {
                // Event abhi chal raha hai, interpolate karo current pos se target pos tak
                let progress = if event_duration > 0.0 {
                    ((t - ev.t) / event_duration).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                state.pos.0 = state.pos.0 + (to.x - state.pos.0) * progress;
                state.pos.1 = state.pos.1 + (to.y - state.pos.1) * progress;
            }
        }
    }

    // 3. Action state sirf latest event se decide hoga
    if let Some(ev) = relevant_events.last() {
        let action_name = ev.action.as_deref().unwrap_or("idle");
        let event_duration = ev.dur.unwrap_or(1.0);
        let time_in_this_event = (t - ev.t).min(event_duration);

        if action_name == "walk" || action_name == "run" {
            state.action = if action_name == "walk" { ActionState::Walk } else { ActionState::Wave };
            state.walk_phase = (time_in_this_event / WALK_CYCLE_PERIOD_SEC) % 1.0;
        } else if action_name == "wave" {
            state.action = ActionState::Wave;
            state.walk_phase = 0.0;
        } else {
            state.action = ActionState::Idle;
            state.walk_phase = 0.0;
        }

        state.action_elapsed = time_in_this_event;
    }

    state
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Background, Character, VideoOpts};

    fn dummy_job_with_timeline(json_timeline: &str) -> Job {
        let json = format!(
            r#"{{
              "job_id": "j1",
              "background": {{ "asset": "bg", "w": 100, "h": 100 }},
              "characters": [
                {{ "id": "c1", "skin": "s", "height": 1.0, "x": 0, "y": 0 }}
              ],
              "timeline": {}
            }}"#,
            json_timeline
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn idle_state_at_zero() {
        let job = dummy_job_with_timeline(r#"[ { "t": 0.0, "char": "c1", "action": "idle" } ]"#);
        let state = get_character_state(&job, "c1", (100, 200), 0.0);
        assert_eq!(state.action, ActionState::Idle);
        assert_eq!(state.pos, (100.0, 200.0));
    }

    #[test]
    fn walk_interpolates_position() {
        let job = dummy_job_with_timeline(
            r#"[ 
                { "t": 0.0, "char": "c1", "action": "idle" },
                { "t": 1.0, "char": "c1", "action": "walk", "to": { "x": 200, "y": 100 }, "dur": 2.0 }
            ]"#,
        );

        // At t=0: idle, pos=(0,0)
        let s0 = get_character_state(&job, "c1", (0, 0), 0.0);
        assert_eq!(s0.action, ActionState::Idle);

        // At t=1.5: 0.5 sec into walk (25% of duration)
        // x = 0 + (200-0)*0.25 = 50
        // y = 0 + (100-0)*0.25 = 25
        let s1 = get_character_state(&job, "c1", (0, 0), 1.5);
        assert_eq!(s1.action, ActionState::Walk);
        assert_eq!(s1.pos, (50.0, 25.0));

        // At t=3.0: end of walk (100%)
        // x = 200, y = 100
        let s2 = get_character_state(&job, "c1", (0, 0), 3.0);
        assert_eq!(s2.action, ActionState::Walk); // walk event end par hai
        assert_eq!(s2.pos, (200.0, 100.0));
    }

    #[test]
    fn walk_phase_cycles() {
        let job = dummy_job_with_timeline(
            r#"[ { "t": 0.0, "char": "c1", "action": "walk", "to": { "x": 100, "y": 0 }, "dur": 2.0 } ]"#,
        );
        // WALK_CYCLE_PERIOD_SEC is 1.0
        // At t=0.25, phase = 0.25
        let s1 = get_character_state(&job, "c1", (0, 0), 0.25);
        assert!((s1.walk_phase - 0.25).abs() < 1e-5);

        // At t=1.25, phase = 1.25 % 1.0 = 0.25
        let s2 = get_character_state(&job, "c1", (0, 0), 1.25);
        assert!((s2.walk_phase - 0.25).abs() < 1e-5);
    }

    #[test]
    fn wave_action_overrides_walk() {
        let job = dummy_job_with_timeline(
            r#"[ 
                { "t": 0.0, "char": "c1", "action": "walk", "to": { "x": 100, "y": 0 }, "dur": 2.0 },
                { "t": 1.0, "char": "c1", "action": "wave", "dur": 1.0 }
            ]"#,
        );
        // At t=1.5, walk chal raha tha, par wave ne override kar diya
        let state = get_character_state(&job, "c1", (0, 0), 1.5);
        assert_eq!(state.action, ActionState::Wave);
    }
}
