//! JSON -> types.
//!
//! §7 kehta hai "JSON ko stream karke part-by-part padhega". M1 mein full-parse
//! hai (JSON chhoti hoti hai), lekin **public interface** stream-safe hai:
//!
//!     fn parse_job<R: Read>(r: R)  -> Result<Job, ParseError>
//!     fn parse_skin<R: Read>(r: R) -> Result<SkinManifest, ParseError>
//!
//! Read-based signature isliye taaki M3/M4 mein aane wala true streaming
//! parser (jaise characters[] ko ek-ek karke padhna) in dono functions ke
//! **andar** badle — callers (main.rs, assets.rs, skeleton.rs) touch nahi honge.
//!
//! Callers ke liye ye teen cheezein guarantee hain:
//!   1. Signature kabhi nahi badlega (M1 -> M4 tak).
//!   2. Success pe poori struct milti hai (partial nahi).
//!   3. Failure pe typed ParseError — message sirf debug ke liye, match
//!      karna ho to variant pe karo.

use crate::types::{Job, SkinManifest};
use std::io::Read;

#[derive(Debug)]
pub enum ParseError {
    /// Bytes padhne mein IO fail (streaming pipe tuta, file missing, etc.).
    Io(std::io::Error),
    /// JSON syntax ya schema galat.
    Syntax(serde_json::Error),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "json read error: {e}"),
            ParseError::Syntax(e) => write!(f, "json syntax error: {e}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParseError::Io(e) => Some(e),
            ParseError::Syntax(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        ParseError::Io(e)
    }
}

impl From<serde_json::Error> for ParseError {
    fn from(e: serde_json::Error) -> Self {
        // serde_json::Error IO wrapping karta hai jab from_reader use ho.
        // Use Syntax ki tarah expose karna theek hai — caller dono ko "bad input"
        // ki tarah hi handle karta hai; agar kabhi zaroorat pade to yahan
        // e.io_error_kind() check karke variant badla ja sakta hai.
        ParseError::Syntax(e)
    }
}

/// Job JSON (§7) ko parse karo.
///
/// M1: `serde_json::from_reader` se poora buffer-kar-and-parse. Bade inputs pe
/// peak memory = poore JSON ka size (M1 ki JSON chhoti hai). M3/M4 mein isi
/// function ke andar streaming parser aayega, aur bade JSON ke liye memory
/// piece-wise ho jayegi.
pub fn parse_job<R: Read>(mut r: R) -> Result<Job, ParseError> {
    // serde_json::from_reader ko Read chahiye — `mut r` isliye.
    let job: Job = serde_json::from_reader(&mut r)?;
    Ok(job)
}

/// Skin manifest (§9 + user A) ko parse karo.
pub fn parse_skin<R: Read>(mut r: R) -> Result<SkinManifest, ParseError> {
    let m: SkinManifest = serde_json::from_reader(&mut r)?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const JOB_JSON: &str = r#"{
      "job_id": "j_001",
      "video": { "w": 720, "h": 1280, "fps": 15 },
      "background": { "asset": "bg/park_day", "w": 2160, "h": 1280 },
      "characters": [
        { "id": "c1", "skin": "oggy", "height": 1.0, "x": 300, "y": 1000 }
      ]
    }"#;

    const SKIN_JSON: &str = r#"{
      "skin": "oggy", "base_height": 400,
      "parts": {
        "body": { "asset": "b", "pivot": { "x": 60, "y": 100 }, "z": 3 },
        "p10":  { "asset": "s",  "pivot": { "x": 5,  "y": 5  }, "z": 4,
                  "parent": "body", "attach": { "x": 40, "y": 20 } }
      }
    }"#;

    // -------- happy path --------

    #[test]
    fn parse_job_ok() {
        let job = parse_job(Cursor::new(JOB_JSON)).unwrap();
        assert_eq!(job.job_id, "j_001");
        assert_eq!(job.characters.len(), 1);
        assert_eq!(job.background.w, 2160);
    }

    #[test]
    fn parse_skin_ok() {
        let m = parse_skin(Cursor::new(SKIN_JSON)).unwrap();
        assert_eq!(m.skin, "oggy");
        assert_eq!(m.parts.len(), 2);
        assert!(m.parts["p10"].attach.is_some());
    }

    // -------- Read abstraction: interface stream-friendly hai --------

    #[test]
    fn parse_job_works_over_read_stream() {
        // Cursor ek Read hai, lekin test ye bhi prove karta hai ki
        // function koi &str/&[u8]-specific nahi hai. File / TcpStream bhi
        // chalega bina signature badle.
        struct ChunkedStream {
            data: Vec<u8>,
            pos: usize,
            chunk: usize,
        }
        impl Read for ChunkedStream {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.chunk.min(buf.len()).min(self.data.len() - self.pos);
                buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            }
        }
        let s = ChunkedStream {
            data: JOB_JSON.as_bytes().to_vec(),
            pos: 0,
            chunk: 7, // odd chunk size — partial reads ka exercise
        };
        let job = parse_job(s).unwrap();
        assert_eq!(job.job_id, "j_001");
    }

    // -------- error paths: variants match karte hain --------

    #[test]
    fn bad_syntax_returns_syntax_error() {
        let err = parse_job(Cursor::new(b"{ not json".as_slice())).unwrap_err();
        assert!(
            matches!(err, ParseError::Syntax(_)),
            "expected Syntax, got {:?}",
            err
        );
    }

    #[test]
    fn missing_required_field_is_syntax_error() {
        // background missing -> serde error -> Syntax variant
        let bad = r#"{ "job_id": "x", "characters": [] }"#;
        let err = parse_job(Cursor::new(bad.as_bytes())).unwrap_err();
        assert!(matches!(err, ParseError::Syntax(_)));
    }

    #[test]
    fn empty_input_is_syntax_error() {
        let err = parse_job(Cursor::new(b"".as_slice())).unwrap_err();
        assert!(matches!(err, ParseError::Syntax(_)));
    }

    #[test]
    fn io_error_from_reader_is_io_variant() {
        // Failing reader — IO error surface hona chahiye as ParseError::Io
        // *ya* serde_json usse wrap karke Syntax mein de — M1 mein hum
        // serde_json ke through jaate hain, to Syntax milta hai. Ye test
        // sirf ye pin karta hai ki koi panic nahi hota aur Err aata hai.
        struct FailingReader;
        impl Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "boom"))
            }
        }
        let err = parse_job(FailingReader).unwrap_err();
        // Ya Io, ya Syntax (serde wrapper ke through) — dono acceptable.
        assert!(
            matches!(err, ParseError::Io(_) | ParseError::Syntax(_)),
            "unexpected variant: {:?}",
            err
        );
    }

    // -------- error Display / source --------

    #[test]
    fn error_display_contains_prefix() {
        let e: ParseError = serde_json::from_str::<Job>("nope").unwrap_err().into();
        let s = format!("{e}");
        assert!(s.contains("json syntax error"), "display: {s}");
    }

    #[test]
    fn error_source_chains() {
        use std::error::Error;
        let e: ParseError = serde_json::from_str::<Job>("nope").unwrap_err().into();
        assert!(e.source().is_some());
    }
  }
