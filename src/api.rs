//! API (§6, §10, M5.0.1): chhota HTTP server, sirf `std::net` se (naya crate nahi).
//!
//! Endpoints:
//!   GET  /health          -> {"ok":true}   (bina key, taaki phone browser se bhi dikhe)
//!   POST /render          -> {"job_id","status":"queued"}   (X-Engine-Key chahiye)
//!   GET  /status/{job_id} -> status, pieces_ready, pieces_total, error   (X-Engine-Key)
//!
//! Render Render.com pe plain HTTP hi chalta hai (HTTPS wo khud sambhalta hai).
//! Ek time pe ek job: ek worker thread, baaki queue mein (max MAX_QUEUE).
//! Piece ready hote hi Telegram pe delivery (delivery.rs), phir local file delete.
//!
//! Secrets environment se: ENGINE_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID, PORT.
//! JSON ke delivery.mode ko yahin check karte hain (Job struct mein ye field nahi).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::config::{Config, DEFAULT_PORT, ENGINE_KEY_ENV, PORT_ENV};
use crate::delivery::{self, TelegramTarget};
use crate::status::{valid_job_id, RegisterResult, StatusBoard};
use crate::types::Job;

/// Queue mein (chal raha job milake) itne se zyada jobs nahi.
pub const MAX_QUEUE: usize = 10;
/// Request body ki max size.
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Request headers ki max size.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Ek job ki max lambai (RAM aur time ki hifazat).
pub const MAX_DURATION_SEC: f32 = 60.0;
pub const MAX_FPS: u32 = 60;
pub const MAX_VIDEO_SIDE: u32 = 2000;

/// Queue mein jane wala job.
pub struct QueuedJob {
    pub job: Job,
    pub pieces_total: u32,
}

/// Server ki shared state.
pub struct Ctx {
    pub key: String,
    pub board: Arc<StatusBoard>,
    pub cfg: Config,
    pub telegram_ready: bool,
    pub pending: Arc<AtomicUsize>,
    pub tx: Sender<QueuedJob>,
}

pub struct Request {
    pub method: String,
    pub path: String,
    /// (naam chhote akshar mein, value)
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    fn json(status: u16, v: Value) -> Self {
        Self { status, body: v.to_string() }
    }
    fn error(status: u16, msg: &str) -> Self {
        Self::json(status, json!({ "error": msg }))
    }
}

// ---------------- HTTP padhna / likhna ----------------

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn bad(msg: &str) -> (u16, String) {
    (400, msg.to_string())
}

/// Ek HTTP request padhta hai (header + body). Galat ya bahut bada ho to (code, message).
pub fn read_request<R: Read>(r: &mut R) -> Result<Request, (u16, String)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];

    let head_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HEAD_BYTES {
            return Err(bad("request headers too large"));
        }
        let n = r.read(&mut chunk).map_err(|e| bad(&format!("read failed: {}", e)))?;
        if n == 0 {
            return Err(bad("connection closed before request end"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or_else(|| bad("bad request line"))?.to_string();
    let target = parts.next().ok_or_else(|| bad("bad request line"))?;
    let path = target.split('?').next().unwrap_or("").to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    let content_length = match headers.iter().find(|(k, _)| k == "content-length") {
        Some((_, v)) => v.parse::<usize>().map_err(|_| bad("bad content-length"))?,
        None => 0,
    };
    if content_length > MAX_BODY_BYTES {
        return Err((413, format!("body too large (max {} bytes)", MAX_BODY_BYTES)));
    }

    let mut body: Vec<u8> = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        let n = r.read(&mut chunk).map_err(|e| bad(&format!("read failed: {}", e)))?;
        if n == 0 {
            return Err(bad("request body incomplete"));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Ok(Request { method, path, headers, body })
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

pub fn write_response<W: Write>(w: &mut W, resp: &Response) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        status_text(resp.status),
        resp.body.len()
    );
    w.write_all(head.as_bytes())?;
    w.write_all(resp.body.as_bytes())?;
    w.flush()
}

// ---------------- Key check ----------------

/// Barabar hone par bhi time same lagta hai (key ka andaza lagana mushkil).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn key_ok(req: &Request, ctx: &Ctx) -> bool {
    match req.header("x-engine-key") {
        Some(k) => constant_time_eq(k.as_bytes(), ctx.key.as_bytes()),
        None => false,
    }
}

// ---------------- Job check ----------------

/// serde ke baad ke hisaab se: fps, duration aur size ki hadein.
pub fn validate_job(job: &Job) -> Result<(), String> {
    if let Some(fps) = job.video.fps {
        if fps == 0 || fps > MAX_FPS {
            return Err(format!("video.fps must be 1..={}", MAX_FPS));
        }
    }
    if let Some(d) = job.video.duration_sec {
        if !(d > 0.0) || d > MAX_DURATION_SEC {
            return Err(format!("video.duration_sec must be > 0 and <= {}", MAX_DURATION_SEC));
        }
    }
    if let Some(p) = job.video.piece_sec {
        if !(p > 0.0) {
            return Err("video.piece_sec must be > 0".to_string());
        }
    }
    for (name, side) in [("w", job.video.w), ("h", job.video.h)] {
        if let Some(s) = side {
            if s < 2 || s > MAX_VIDEO_SIDE {
                return Err(format!("video.{} must be 2..={}", name, MAX_VIDEO_SIDE));
            }
        }
    }
    Ok(())
}

// ---------------- Routing ----------------

pub fn route(req: &Request, ctx: &Ctx) -> Response {
    let path = req.path.trim_end_matches('/');
    match (req.method.as_str(), path) {
        ("GET", "/health") => Response::json(200, json!({ "ok": true })),
        ("POST", "/render") => {
            if !key_ok(req, ctx) {
                return Response::error(401, "missing or wrong X-Engine-Key");
            }
            handle_render(req, ctx)
        }
        ("GET", p) if p.starts_with("/status/") => {
            if !key_ok(req, ctx) {
                return Response::error(401, "missing or wrong X-Engine-Key");
            }
            let id = &p["/status/".len()..];
            if !valid_job_id(id) {
                return Response::error(400, "invalid job_id");
            }
            match ctx.board.get(id) {
                Some(s) => Response::json(200, s.to_json()),
                None => Response::error(404, "unknown job"),
            }
        }
        _ => Response::error(404, "not found"),
    }
}

fn handle_render(req: &Request, ctx: &Ctx) -> Response {
    if !ctx.telegram_ready {
        return Response::error(
            503,
            "telegram delivery is not configured (set TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID)",
        );
    }

    // delivery.mode Job struct mein nahi hai, isliye raw JSON se padhte hain.
    let raw: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(e) => return Response::error(400, &format!("invalid JSON: {}", e)),
    };
    let mode = raw["delivery"]["mode"].as_str().unwrap_or("default_storage");
    if let Err(e) = delivery::check_mode(mode) {
        return Response::error(400, &e);
    }

    let job: Job = match serde_json::from_slice(&req.body) {
        Ok(j) => j,
        Err(e) => return Response::error(400, &format!("invalid job: {}", e)),
    };
    if !valid_job_id(&job.job_id) {
        return Response::error(400, "invalid job_id (letters, digits, _ and - only)");
    }
    if let Err(e) = validate_job(&job) {
        return Response::error(400, &e);
    }

    if ctx.pending.load(Ordering::SeqCst) >= MAX_QUEUE {
        return Response::error(503, "queue is full, try again later");
    }

    let pieces_total = crate::count_pieces(&job, &ctx.cfg);
    let job_id = job.job_id.clone();
    match ctx.board.register(&job_id, pieces_total) {
        Err(e) => Response::error(400, &e),
        Ok(RegisterResult::Existing(state)) => Response::json(
            200,
            json!({ "job_id": job_id, "status": state.as_str(), "note": "already known, not restarted" }),
        ),
        Ok(RegisterResult::New) => {
            ctx.pending.fetch_add(1, Ordering::SeqCst);
            if ctx.tx.send(QueuedJob { job, pieces_total }).is_err() {
                ctx.pending.fetch_sub(1, Ordering::SeqCst);
                ctx.board.set_failed(&job_id, "worker is not running");
                return Response::error(500, "worker is not running");
            }
            Response::json(200, json!({ "job_id": job_id, "status": "queued" }))
        }
    }
}

// ---------------- Worker (ek time pe ek job) ----------------

fn run_one(
    q: &QueuedJob,
    cfg: &Config,
    assets_root: &Path,
    out_dir: &Path,
    target: &TelegramTarget,
    board: &StatusBoard,
) -> Result<(), String> {
    let id = q.job.job_id.clone();
    let total = q.pieces_total;
    crate::render_job_pieces(
        &q.job,
        cfg,
        assets_root,
        out_dir,
        &mut |piece: u32, path: &Path| -> Result<(), String> {
            let caption = delivery::caption_for(&id, piece, total);
            delivery::deliver_piece(cfg, target, path, &caption)?;
            board.piece_ready(&id);
            Ok(())
        },
    )?;
    Ok(())
}

fn worker_loop(
    rx: Receiver<QueuedJob>,
    cfg: Config,
    assets_root: PathBuf,
    work_dir: PathBuf,
    target: Option<TelegramTarget>,
    board: Arc<StatusBoard>,
    pending: Arc<AtomicUsize>,
) {
    for q in rx {
        let id = q.job.job_id.clone();
        board.set_rendering(&id);
        let out_dir = work_dir.join(&id);
        let _ = std::fs::remove_dir_all(&out_dir); // purane adhoore pieces

        let outcome: Result<(), String> = match &target {
            None => Err("telegram is not configured".to_string()),
            Some(t) => {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_one(&q, &cfg, &assets_root, &out_dir, t, &board)
                }));
                match r {
                    Ok(res) => res,
                    Err(_) => Err("internal error: render panicked".to_string()),
                }
            }
        };

        let _ = std::fs::remove_dir_all(&out_dir); // disk temporary hai (§13)
        match outcome {
            Ok(()) => board.set_done(&id),
            Err(e) => {
                eprintln!("job {} failed: {}", id, e);
                board.set_failed(&id, &e);
            }
        }
        pending.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------- Server ----------------

fn handle_connection(mut stream: TcpStream, ctx: &Ctx) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let resp = match read_request(&mut stream) {
        Ok(req) => {
            let resp = route(&req, ctx);
            println!("{} {} -> {}", req.method, req.path, resp.status);
            resp
        }
        Err((code, msg)) => Response::error(code, &msg),
    };
    let _ = write_response(&mut stream, &resp);
}

/// Server chalata hai (wapas nahi aata jab tak band na ho).
pub fn serve(cfg: Config, assets_root: PathBuf, work_dir: PathBuf) -> Result<(), String> {
    let key = std::env::var(ENGINE_KEY_ENV).unwrap_or_default();
    if key.trim().is_empty() {
        return Err(format!(
            "{} is not set; refusing to start without a shared secret",
            ENGINE_KEY_ENV
        ));
    }

    let target = match TelegramTarget::from_env() {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("warning: telegram not configured: {}", e);
            None
        }
    };
    if !crate::encoder::ffmpeg_available(cfg.ffmpeg_path) {
        eprintln!("warning: ffmpeg not found ('{}'), renders will fail", cfg.ffmpeg_path);
    }

    std::fs::create_dir_all(&work_dir)
        .map_err(|e| format!("create work_dir {}: {}", work_dir.display(), e))?;

    let port = std::env::var(PORT_ENV)
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let listener = TcpListener::bind(("0.0.0.0", port))
        .map_err(|e| format!("bind port {}: {}", port, e))?;

    let board = Arc::new(StatusBoard::new());
    let pending = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel::<QueuedJob>();

    {
        let (cfg, assets_root, work_dir) = (cfg, assets_root.clone(), work_dir.clone());
        let (target, board, pending) = (target.clone(), board.clone(), pending.clone());
        std::thread::spawn(move || {
            worker_loop(rx, cfg, assets_root, work_dir, target, board, pending)
        });
    }

    let ctx = Ctx {
        key,
        board,
        cfg,
        telegram_ready: target.is_some(),
        pending,
        tx,
    };

    println!("lkj-engine listening on 0.0.0.0:{}", port);
    for stream in listener.incoming() {
        match stream {
            Ok(s) => handle_connection(s, &ctx),
            Err(e) => eprintln!("accept failed: {}", e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_JOB: &str = r#"{
        "job_id": "j_001",
        "video": { "w": 200, "h": 200, "fps": 15, "duration_sec": 10.0 },
        "background": { "asset": "bg", "w": 400, "h": 400 },
        "characters": [ { "id": "c1", "skin": "oggy", "height": 1.0, "x": 100, "y": 100 } ],
        "timeline": [ { "t": 0.0, "char": "c1", "action": "idle" } ],
        "delivery": { "mode": "default_storage" }
    }"#;

    fn test_ctx(telegram_ready: bool) -> (Ctx, Receiver<QueuedJob>) {
        let (tx, rx) = mpsc::channel();
        let ctx = Ctx {
            key: "secret".to_string(),
            board: Arc::new(StatusBoard::new()),
            cfg: Config::default(),
            telegram_ready,
            pending: Arc::new(AtomicUsize::new(0)),
            tx,
        };
        (ctx, rx)
    }

    fn req(method: &str, path: &str, key: Option<&str>, body: &str) -> Request {
        let mut headers = Vec::new();
        if let Some(k) = key {
            headers.push(("x-engine-key".to_string(), k.to_string()));
        }
        Request {
            method: method.to_string(),
            path: path.to_string(),
            headers,
            body: body.as_bytes().to_vec(),
        }
    }

    fn body_json(r: &Response) -> Value {
        serde_json::from_str(&r.body).unwrap()
    }

    #[test]
    fn read_request_parses_post_with_body() {
        let raw = b"POST /render?x=1 HTTP/1.1\r\nHost: a\r\nX-Engine-Key: k1\r\nContent-Length: 5\r\n\r\nhello";
        let r = read_request(&mut &raw[..]).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/render");
        assert_eq!(r.header("x-engine-key"), Some("k1"));
        assert_eq!(r.body, b"hello");
    }

    #[test]
    fn read_request_rejects_big_body_and_garbage() {
        let big = format!(
            "POST /render HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        let e = read_request(&mut big.as_bytes()).err().unwrap();
        assert_eq!(e.0, 413);

        let short = b"POST /render HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc";
        assert_eq!(read_request(&mut &short[..]).err().unwrap().0, 400);

        let none = b"";
        assert_eq!(read_request(&mut &none[..]).err().unwrap().0, 400);
    }

    #[test]
    fn health_is_open() {
        let (ctx, _rx) = test_ctx(true);
        let r = route(&req("GET", "/health", None, ""), &ctx);
        assert_eq!(r.status, 200);
        assert_eq!(body_json(&r)["ok"], true);
    }

    #[test]
    fn render_and_status_need_key() {
        let (ctx, rx) = test_ctx(true);
        assert_eq!(route(&req("POST", "/render", None, GOOD_JOB), &ctx).status, 401);
        assert_eq!(route(&req("POST", "/render", Some("wrong"), GOOD_JOB), &ctx).status, 401);
        assert_eq!(route(&req("GET", "/status/j_001", None, ""), &ctx).status, 401);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn render_accepts_valid_job_and_queues_it() {
        let (ctx, rx) = test_ctx(true);
        let r = route(&req("POST", "/render", Some("secret"), GOOD_JOB), &ctx);
        assert_eq!(r.status, 200);
        let v = body_json(&r);
        assert_eq!(v["job_id"], "j_001");
        assert_eq!(v["status"], "queued");
        let q = rx.try_recv().expect("job must be queued");
        assert_eq!(q.job.job_id, "j_001");
        assert_eq!(q.pieces_total, 2); // 10 sec / 5 sec
        assert_eq!(ctx.pending.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn same_job_twice_is_not_requeued() {
        let (ctx, rx) = test_ctx(true);
        route(&req("POST", "/render", Some("secret"), GOOD_JOB), &ctx);
        let r = route(&req("POST", "/render", Some("secret"), GOOD_JOB), &ctx);
        assert_eq!(r.status, 200);
        assert_eq!(body_json(&r)["status"], "queued");
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err(), "second submit must not queue again");
    }

    #[test]
    fn render_rejects_url_mode_and_bad_input() {
        let (ctx, rx) = test_ctx(true);
        let url_job = GOOD_JOB.replace("default_storage", "url");
        let r = route(&req("POST", "/render", Some("secret"), &url_job), &ctx);
        assert_eq!(r.status, 400);
        assert!(body_json(&r)["error"].as_str().unwrap().contains("not supported"));

        assert_eq!(route(&req("POST", "/render", Some("secret"), "not json"), &ctx).status, 400);

        let bad_id = GOOD_JOB.replace("j_001", "../x");
        assert_eq!(route(&req("POST", "/render", Some("secret"), &bad_id), &ctx).status, 400);

        let long = GOOD_JOB.replace("10.0", "9999.0");
        assert_eq!(route(&req("POST", "/render", Some("secret"), &long), &ctx).status, 400);

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn render_without_telegram_is_503() {
        let (ctx, rx) = test_ctx(false);
        let r = route(&req("POST", "/render", Some("secret"), GOOD_JOB), &ctx);
        assert_eq!(r.status, 503);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn full_queue_is_503() {
        let (ctx, rx) = test_ctx(true);
        ctx.pending.store(MAX_QUEUE, Or
