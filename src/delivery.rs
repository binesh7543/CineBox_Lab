//! Delivery (§10, §13, M5.0.1): piece ki MP4 Telegram group mein bhejna.
//!
//! Abhi sirf Telegram. JSON ka delivery.mode "default_storage" = Telegram maana
//! jata hai; "url" mode baad mein (M5.1), abhi saaf error deta hai.
//!
//! Bot token aur chat id Render ke environment se aate hain (config.rs mein sirf
//! unke naam hain). Token kabhi error message ya log mein nahi jata (redact).
//!
//! HTTPS ke liye `ureq` (Lkj ne approve kiya). ureq multipart khud nahi karta,
//! isliye body yahin haath se banti hai. Piece file memory mein padhi jati hai
//! (5 sec ka piece chhota hota hai), phir bhejne ke baad local file delete.
//!
//! Retry: `Config::delivery_retries` = kul koshishein (3), beech mein thoda wait.

use std::path::Path;
use std::time::Duration;

use crate::config::{Config, TELEGRAM_BOT_TOKEN_ENV, TELEGRAM_CHAT_ID_ENV};

/// Telegram Bot API ki upload limit.
pub const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Do koshishon ke beech base wait (koshish number se guna hota hai).
const RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const BOUNDARY: &str = "----lkjEngineBoundary7MA4YWxkTrZu0gW";
/// Telegram ke error text ka max hissa.
const MAX_ERR_CHARS: usize = 300;

/// Kahan bhejna hai. Token ko Debug mein chhupa ke rakha hai.
#[derive(Clone)]
pub struct TelegramTarget {
    token: String,
    chat_id: String,
}

impl std::fmt::Debug for TelegramTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramTarget")
            .field("token", &"***")
            .field("chat_id", &self.chat_id)
            .finish()
    }
}

impl TelegramTarget {
    /// Environment se token aur chat id padhta hai.
    pub fn from_env() -> Result<Self, String> {
        Self::from_values(
            std::env::var(TELEGRAM_BOT_TOKEN_ENV).ok(),
            std::env::var(TELEGRAM_CHAT_ID_ENV).ok(),
        )
    }

    /// Values se banata hai (test ke liye alag, env ke bina).
    pub fn from_values(token: Option<String>, chat_id: Option<String>) -> Result<Self, String> {
        let token = token.unwrap_or_default().trim().to_string();
        if token.is_empty() {
            return Err(format!("{} is not set", TELEGRAM_BOT_TOKEN_ENV));
        }
        if !token.contains(':') {
            return Err(format!("{} does not look like a bot token", TELEGRAM_BOT_TOKEN_ENV));
        }
        let chat_id = chat_id.unwrap_or_default().trim().to_string();
        if chat_id.is_empty() {
            return Err(format!("{} is not set", TELEGRAM_CHAT_ID_ENV));
        }
        if !valid_chat_id(&chat_id) {
            return Err(format!(
                "{} must be a number (groups are negative) or @channelname",
                TELEGRAM_CHAT_ID_ENV
            ));
        }
        Ok(Self { token, chat_id })
    }

    fn send_video_url(&self) -> String {
        format!("https://api.telegram.org/bot{}/sendVideo", self.token)
    }
}

/// Chat id: -1001234567890 jaisa number, ya @channelname.
fn valid_chat_id(id: &str) -> bool {
    if let Some(name) = id.strip_prefix('@') {
        return !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    }
    let digits = id.strip_prefix('-').unwrap_or(id);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// JSON ka delivery.mode check. Abhi sirf default_storage (= Telegram).
pub fn check_mode(mode: &str) -> Result<(), String> {
    match mode {
        "default_storage" => Ok(()),
        "url" => Err("delivery.mode \"url\" is not supported yet (M5.1); use \"default_storage\"".to_string()),
        other => Err(format!("unknown delivery.mode \"{}\"", other)),
    }
}

/// Telegram mein dikhne wala caption, jaise "job j_001 piece 2/4".
pub fn caption_for(job_id: &str, piece: u32, total: u32) -> String {
    format!("job {} piece {}/{}", job_id, piece + 1, total)
}

/// File ka size limit ke andar hai ya nahi.
fn check_size(len: u64) -> Result<(), String> {
    if len > MAX_UPLOAD_BYTES {
        return Err(format!(
            "piece is {} bytes, Telegram limit is {} bytes",
            len, MAX_UPLOAD_BYTES
        ));
    }
    Ok(())
}

/// Error text se token hata do (ureq ke error mein URL aa sakta hai).
fn redact(text: &str, token: &str) -> String {
    if token.is_empty() {
        return text.to_string();
    }
    text.replace(token, "***")
}

fn truncate(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= MAX_ERR_CHARS {
        t.to_string()
    } else {
        t.chars().take(MAX_ERR_CHARS).collect()
    }
}

/// multipart/form-data body: chat_id, caption, supports_streaming aur video file.
pub fn build_multipart(chat_id: &str, caption: &str, file_name: &str, file_bytes: &[u8]) -> Vec<u8> {
    let safe_name: String = file_name
        .chars()
        .map(|c| if c == '"' || c == '\r' || c == '\n' { '_' } else { c })
        .collect();

    let mut body: Vec<u8> = Vec::with_capacity(file_bytes.len() + 1024);
    let mut text_field = |name: &str, value: &str| {
        body.extend_from_slice(format!("--{}\r\n", BOUNDARY).as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name).as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    };
    text_field("chat_id", chat_id);
    if !caption.is_empty() {
        text_field("caption", caption);
    }
    text_field("supports_streaming", "true");

    body.extend_from_slice(format!("--{}\r\n", BOUNDARY).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"video\"; filename=\"{}\"\r\n",
            safe_name
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: video/mp4\r\n\r\n");
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{}--\r\n", BOUNDARY).as_bytes());
    body
}

/// Ek koshish: body Telegram ko POST karo. Ok tabhi jab Telegram "ok": true de.
fn post_once(target: &TelegramTarget, body: &[u8]) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build();

    let result = agent
        .post(&target.send_video_url())
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={}", BOUNDARY),
        )
        .send_bytes(body);

    let outcome: Result<(), String> = match result {
        Ok(resp) => match resp.into_string() {
            Ok(text) => {
                let ok = serde_json::from_str::<serde_json::Value>(&text)
                    .map(|v| v["ok"] == true)
                    .unwrap_or(false);
                if ok {
                    Ok(())
                } else {
                    Err(format!("telegram replied without ok: {}", truncate(&text)))
                }
            }
            Err(e) => Err(format!("telegram reply unreadable: {}", e)),
        },
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Err(format!("telegram http {}: {}", code, truncate(&text)))
        }
        Err(e) => Err(format!("telegram request failed: {}", e)),
    };
    outcome.map_err(|m| redact(&m, &target.token))
}

/// `attempts` baar tak koshish, beech mein base_delay * koshish_number wait.
pub fn retry<F>(attempts: u32, base_delay: Duration, mut f: F) -> Result<(), String>
where
    F: FnMut() -> Result<(), String>,
{
    let attempts = attempts.max(1);
    let mut last = String::new();
    for i in 1..=attempts {
        match f() {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e;
                if i < attempts {
                    std::thread::sleep(base_delay * i);
                }
            }
        }
    }
    Err(format!("failed after {} attempts: {}", attempts, last))
}

/// Piece Telegram pe bhejo (retry ke saath). Kamyab hone par local file delete.
pub fn deliver_piece(
    cfg: &Config,
    target: &TelegramTarget,
    path: &Path,
    caption: &str,
) -> Result<(), String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("cannot read piece {}: {}", path.display(), e))?
        .len();
    check_size(len)?;
    let bytes = std::fs::read(path)
        .map_err(|e| format!("cannot read piece {}: {}", path.display(), e))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "piece.mp4".to_string());
    let body = build_multipart(&target.chat_id, caption, &name, &bytes);
    drop(bytes);

    retry(cfg.delivery_retries, RETRY_BASE_DELAY, || post_once(target, &body))?;

    // §13: bhejne ke turant baad local file delete (disk temporary hai).
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn multipart_has_fields_and_file() {
        let file = [0u8, 255, 10, 13, 66, 77];
        let body = build_multipart("-100123", "job j1 piece 1/4", "piece_000.mp4", &file);
        assert!(contains(&body, b"name=\"chat_id\"\r\n\r\n-100123\r\n"));
        assert!(contains(&body, b"name=\"caption\"\r\n\r\njob j1 piece 1/4\r\n"));
        assert!(contains(&body, b"name=\"supports_streaming\"\r\n\r\ntrue\r\n"));
        assert!(contains(&body, b"name=\"video\"; filename=\"piece_000.mp4\""));
        assert!(contains(&body, b"Content-Type: video/mp4\r\n\r\n"));
        assert!(contains(&body, &file));
        let end = format!("--{}--\r\n", BOUNDARY);
        assert!(body.ends_with(end.as_bytes()));
    }

    #[test]
    fn multipart_skips_empty_caption_and_cleans_filename() {
        let body = build_multipart("-100123", "", "a\"b.mp4", &[1, 2, 3]);
        assert!(!contains(&body, b"name=\"caption\""));
        assert!(contains(&body, b"filename=\"a_b.mp4\""));
    }

    #[test]
    fn target_from_values_ok() {
        let t = TelegramTarget::from_values(
            Some("123456:ABC-def".to_string()),
            Some("-1001234567890".to_string()),
        )
        .unwrap();
        assert_eq!(t.chat_id, "-1001234567890");
        assert!(t.send_video_url().ends_with("/bot123456:ABC-def/sendVideo"));
    }

    #[test]
    fn target_rejects_missing_or_bad_values() {
        assert!(TelegramTarget::from_values(None, Some("-100".to_string())).is_err());
        assert!(TelegramTarget::from_values(Some("nocolon".to_string()), Some("-100".to_string())).is_err());
        assert!(TelegramTarget::from_values(Some("1:A".to_string()), None).is_err());
        assert!(TelegramTarget::from_values(Some("1:A".to_string()), Some("abc".to_string())).is_err());
    }

    #[test]
    fn debug_hides_token() {
        let t = TelegramTarget::from_values(Some("999:SECRET".to_string()), Some("-1".to_string())).unwrap();
        let shown = format!("{:?}", t);
        assert!(!shown.contains("SECRET"));
    }

    #[test]
    fn chat_id_formats() {
        assert!(valid_chat_id("-1001234567890"));
        assert!(valid_chat_id("12345"));
        assert!(valid_chat_id("@my_channel"));
        assert!(!valid_chat_id(""));
        assert!(!valid_chat_id("-"));
        assert!(!valid_chat_id("@"));
        assert!(!valid_chat_id("12a"));
    }

    #[test]
    fn mode_check() {
        assert!(check_mode("default_storage").is_ok());
        assert!(check_mode("url").unwrap_err().contains("not supported"));
        assert!(check_mode("ftp").is_err());
    }

    #[test]
    fn caption_is_one_based() {
        assert_eq!(caption_for("j_001", 0, 4), "job j_001 piece 1/4");
        assert_eq!(caption_for("j_001", 3, 4), "job j_001 piece 4/4");
    }

    #[test]
    fn size_limit() {
        assert!(check_size(1024).is_ok());
        assert!(check_size(MAX_UPLOAD_BYTES).is_ok());
        assert!(check_size(MAX_UPLOAD_BYTES + 1).is_err());
    }

    #[test]
    fn redact_hides_token() {
        let out = redact("POST https://api.telegram.org/bot123:ABC/sendVideo failed", "123:ABC");
        assert!(!out.contains("123:ABC"));
        assert!(out.contains("***"));
    }

    #[test]
    fn retry_succeeds_on_second_try() {
        let mut calls = 0;
        let r = retry(3, Duration::from_millis(0), || {
            calls += 1;
            if calls < 2 {
                Err("temporary".to_string())
            } else {
                Ok(())
            }
        });
        assert!(r.is_ok());
        assert_eq!(calls, 2);
    }

    #[test]
    fn retry_gives_up_after_attempts() {
        let mut calls = 0;
        let r = retry(3, Duration::from_millis(0), || {
            calls += 1;
            Err("down".to_string())
        });
        assert_eq!(calls, 3);
        let msg = r.unwrap_err();
        assert!(msg.contains("3 attempts"));
        assert!(msg.contains("down"));
    }

    #[test]
    fn missing_piece_file_fails_fast() {
        let cfg = Config::default();
        let t = TelegramTarget::from_values(Some("1:A".to_string()), Some("-1".to_string())).unwrap();
        let r = deliver_piece(&cfg, &t, Path::new("/definitely/not/here/piece_000.mp4"), "x");
        assert!(r.unwrap_err().contains("cannot read piece"));
    }
}
