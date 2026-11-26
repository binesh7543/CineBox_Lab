use lkj_engine::types::Job;
use lkj_engine::timeline::{get_character_state, ActionState};

#[test]
fn position_held_after_walk() {
    let json = r#"{
      "job_id": "j1",
      "background": { "asset": "bg", "w": 100, "h": 100 },
      "characters": [
        { "id": "c1", "skin": "s", "height": 1.0, "x": 300, "y": 1000 }
      ],
      "timeline": [
        { "t": 2.0, "char": "c1", "action": "walk", "to": { "x": 600, "y": 1000 }, "dur": 3.0 },
        { "t": 5.0, "char": "c1", "action": "wave" }
      ]
    }"#;

    let job: Job = serde_json::from_str(json).unwrap();

    // t = 5.5 par (Wave ke beech mein)
    let state = get_character_state(&job, "c1", (300, 1000), 5.5);

    // Action wave hona chahiye
    assert_eq!(state.action, ActionState::Wave);

    // Position walk ke end (600, 1000) par ruki honi chahiye, reset hoke (300, 1000) nahi honi chahiye
    assert_eq!(state.pos, (600.0, 1000.0));
}

#[test]
fn position_held_after_face_event() {
    // Same logic, par walk ke baad face event hai
    let json = r#"{
      "job_id": "j1",
      "background": { "asset": "bg", "w": 100, "h": 100 },
      "characters": [
        { "id": "c1", "skin": "s", "height": 1.0, "x": 300, "y": 1000 }
      ],
      "timeline": [
        { "t": 2.0, "char": "c1", "action": "walk", "to": { "x": 600, "y": 1000 }, "dur": 3.0 },
        { "t": 5.0, "char": "c1", "face": { "expr": "happy", "look": "right" } }
      ]
    }"#;

    let job: Job = serde_json::from_str(json).unwrap();

    let state = get_character_state(&job, "c1", (300, 1000), 5.5);

    // Position reset nahi honi chahiye
    assert_eq!(state.pos, (600.0, 1000.0));
}
