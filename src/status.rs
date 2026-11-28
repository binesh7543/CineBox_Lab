//! Status (§10, §6, M5.0.1): job ka progress aur error track karna.
//!
//! GET /status/{job_id} ka jawab yahin se banta hai: status (queued, rendering,
//! done, failed), kitne pieces ready, aur error message. Sirf `std` + serde_json.
//!
//! `StatusBoard` ko `Arc` mein rakh ke API thread aur render thread dono share
//! karenge. Memory ke liye sirf pichle MAX_JOBS_KEPT jobs yaad rakhe jate hain.
//!
//! Resume (engine restart ke baad) M5.1 mein aayega. Abhi: wahi job_id dobara
//! aaye aur wo failed ho to naye sire se queue hota hai; queued/rendering/done ho
//! to dobara shuru nahi hota (§6).

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};

/// Itne se zyada jobs ka status yaad nahi rakhte (purane khatam jobs hatate hain).
pub const MAX_JOBS_KEPT: usize = 100;
/// job_id ki max lambai.
pub const MAX_JOB_ID_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Rendering,
    Done,
    Failed,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Rendering => "rendering",
            JobState::Done => "done",
            JobState::Failed => "failed",
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, JobState::Done | JobState::Failed)
    }
}

#[derive(Debug, Clone)]
pub struct JobStatus {
    pub job_id: String,
    pub state: JobState,
    pub pieces_total: u32,
    pub pieces_ready: u32,
    pub error: Option<String>,
}

impl JobStatus {
    /// GET /status/{job_id} ka JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "job_id": self.job_id,
            "status": self.state.as_str(),
            "pieces_ready": self.pieces_ready,
            "pieces_total": self.pieces_total,
            "error": self.error,
        })
    }
}

/// register() ka nateeja.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterResult {
    /// Naya job queue mein gaya (ya failed job dobara queue hua).
    New,
    /// Ye job_id pehle se hai (queued, rendering ya done): dobara shuru nahi hoga.
    Existing(JobState),
}

/// job_id file path mein bhi use hota hai, isliye sirf safe characters.
pub fn valid_job_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_JOB_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Video ke liye kitne pieces banenge (duration / piece_sec, upar ki taraf, kam se kam 1).
pub fn pieces_for_duration(duration_sec: f32, piece_sec: f32) -> u32 {
    if piece_sec <= 0.0 || duration_sec <= 0.0 {
        return 1;
    }
    ((duration_sec / piece_sec).ceil() as u32).max(1)
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<String, JobStatus>,
    /// Aane ka order, purane khatam jobs hatane ke liye.
    order: Vec<String>,
}

/// Saare jobs ka status. Thread-safe.
#[derive(Default)]
pub struct StatusBoard {
    inner: Mutex<Inner>,
}

impl StatusBoard {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Kisi thread ke panic se lock poison ho jaye to bhi status padhna chalu rahe.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Naya job darj karta hai. Galat job_id ya pehle se maujood job pe New nahi deta.
    pub fn register(&self, job_id: &str, pieces_total: u32) -> Result<RegisterResult, String> {
        if !valid_job_id(job_id) {
            return Err(format!(
                "invalid job_id (use letters, digits, _ and -, max {} chars)",
                MAX_JOB_ID_LEN
            ));
        }
        let mut g = self.lock();
        if let Some(existing) = g.jobs.get(job_id) {
            if existing.state != JobState::Failed {
                return Ok(RegisterResult::Existing(existing.state));
            }
        }
        let fresh = JobStatus {
            job_id: job_id.to_string(),
            state: JobState::Queued,
            pieces_total,
            pieces_ready: 0,
            error: None,
        };
        if g.jobs.insert(job_id.to_string(), fresh).is_none() {
            g.order.push(job_id.to_string());
        }
        Self::trim(&mut g);
        Ok(RegisterResult::New)
    }

    /// Bahut jobs ho jayein to sabse purana khatam (done/failed) job hata do.
    fn trim(g: &mut Inner) {
        while g.jobs.len() > MAX_JOBS_KEPT {
            let pos = g
                .order
                .iter()
                .position(|id| g.jobs.get(id).map(|j| j.state.is_finished()).unwrap_or(true));
            match pos {
                Some(p) => {
                    let id = g.order.remove(p);
                    g.jobs.remove(&id);
                }
                // Sab chal rahe hain: kuch nahi hata sakte.
                None => break,
            }
        }
    }

    pub fn set_rendering(&self, job_id: &str) {
        if let Some(j) = self.lock().jobs.get_mut(job_id) {
            j.state = JobState::Rendering;
        }
    }

    /// Ek aur piece ready (aur delivery ho chuka).
    pub fn piece_ready(&self, job_id: &str) {
        if let Some(j) = self.lock().jobs.get_mut(job_id) {
            if j.pieces_ready < j.pieces_total {
                j.pieces_ready += 1;
            }
        }
    }

    pub fn set_done(&self, job_id: &str) {
        if let Some(j) = self.lock().jobs.get_mut(job_id) {
            j.state = JobState::Done;
            j.error = None;
        }
    }

    pub fn set_failed(&self, job_id: &str, message: &str) {
        if let Some(j) = self.lock().jobs.get_mut(job_id) {
            j.state = JobState::Failed;
            j.error = Some(message.to_string());
        }
    }

    pub fn get(&self, job_id: &str) -> Option<JobStatus> {
        self.lock().jobs.get(job_id).cloned()
    }

    /// Kitne jobs ka status yaad hai (test aur /health ke liye).
    pub fn len(&self) -> usize {
        self.lock().jobs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_new_then_existing() {
        let b = StatusBoard::new();
        assert_eq!(b.register("j_001", 4), Ok(RegisterResult::New));
        assert_eq!(
            b.register("j_001", 4),
            Ok(RegisterResult::Existing(JobState::Queued))
        );
        b.set_rendering("j_001");
        assert_eq!(
            b.register("j_001", 4),
            Ok(RegisterResult::Existing(JobState::Rendering))
        );
    }

    #[test]
    fn progress_flow() {
        let b = StatusBoard::new();
        b.register("j1", 2).unwrap();
        assert_eq!(b.get("j1").unwrap().state, JobState::Queued);
        b.set_rendering("j1");
        b.piece_ready("j1");
        let s = b.get("j1").unwrap();
        assert_eq!(s.state, JobState::Rendering);
        assert_eq!(s.pieces_ready, 1);
        b.piece_ready("j1");
        b.piece_ready("j1"); // total se zyada nahi badhna chahiye
        b.set_done("j1");
        let s = b.get("j1").unwrap();
        assert_eq!(s.state, JobState::Done);
        assert_eq!(s.pieces_ready, 2);
    }

    #[test]
    fn failed_keeps_error() {
        let b = StatusBoard::new();
        b.register("j1", 4).unwrap();
        b.set_failed("j1", "ffmpeg exited with 1");
        let s = b.get("j1").unwrap();
        assert_eq!(s.state, JobState::Failed);
        assert_eq!(s.error.as_deref(), Some("ffmpeg exited with 1"));
    }

    #[test]
    fn failed_job_can_be_registered_again() {
        let b = StatusBoard::new();
        b.register("j1", 4).unwrap();
        b.piece_ready("j1");
        b.set_failed("j1", "boom");
        assert_eq!(b.register("j1", 4), Ok(RegisterResult::New));
        let s = b.get("j1").unwrap();
        assert_eq!(s.state, JobState::Queued);
        assert_eq!(s.pieces_ready, 0);
        assert_eq!(s.error, None);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn unknown_job_is_none() {
        let b = StatusBoard::new();
        assert!(b.get("nope").is_none());
    }

    #[test]
    fn to_json_shape() {
        let b = StatusBoard::new();
        b.register("j_001", 4).unwrap();
        b.set_rendering("j_001");
        b.piece_ready("j_001");
        let v = b.get("j_001").unwrap().to_json();
        assert_eq!(v["job_id"], "j_001");
        assert_eq!(v["status"], "rendering");
        assert_eq!(v["pieces_ready"], 1);
        assert_eq!(v["pieces_total"], 4);
        assert!(v["error"].is_null());
    }

    #[test]
    fn job_id_validation() {
        assert!(valid_job_id("j_001"));
        assert!(valid_job_id("job-2026-11-27"));
        assert!(!valid_job_id(""));
        assert!(!valid_job_id("../etc/passwd"));
        assert!(!valid_job_id("a b"));
        assert!(!valid_job_id(&"a".repeat(MAX_JOB_ID_LEN + 1)));
        let b = StatusBoard::new();
        assert!(b.register("../x", 1).is_err());
    }

    #[test]
    fn pieces_for_duration_rounds_up() {
        assert_eq!(pieces_for_duration(20.0, 5.0), 4);
        assert_eq!(pieces_for_duration(21.0, 5.0), 5);
        assert_eq!(pieces_for_duration(3.0, 5.0), 1);
        assert_eq!(pieces_for_duration(0.0, 5.0), 1);
    }

    #[test]
    fn old_finished_jobs_are_trimmed() {
        let b = StatusBoard::new();
        for i in 0..(MAX_JOBS_KEPT + 5) {
            let id = format!("j{}", i);
            b.register(&id, 1).unwrap();
            b.set_done(&id);
        }
        assert!(b.len() <= MAX_JOBS_KEPT);
        // sabse purana gaya, sabse naya bacha.
        assert!(b.get("j0").is_none());
        assert!(b.get(&format!("j{}", MAX_JOBS_KEPT + 4)).is_some());
    }

    #[test]
    fn running_jobs_are_never_trimmed() {
        let b = StatusBoard::new();
        for i in 0..(MAX_JOBS_KEPT + 3) {
            b.register(&format!("j{}", i), 1).unwrap();
        }
        // Sab queued hain, kuch bhi hata nahi sakte.
        assert_eq!(b.len(), MAX_JOBS_KEPT + 3);
    }
}
