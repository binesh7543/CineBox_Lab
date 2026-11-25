//! Asset loading, PNG decode, scaling, aur LRU cache (§9, §11).
//!
//! Interface contract (M1 se M5 tak sthir):
//!
//!     AssetLoader::new(root: PathBuf, max_cache_bytes: usize) -> Self
//!     AssetLoader::load(&mut self, name: &str, scale: f32)
//!         -> Result<Arc<CachedImage>, AssetError>
//!
//! - `name` = skin manifest ka `asset` field (jaise "oggy/part_6"). Disk pe
//!   `<root>/<name>.png` padha jaata hai. M2 mein URL fallback isi function
//!   ke andar aayega, callers touch nahi honge.
//! - `scale` = character height (base_height pe authored skin, §9). 1.0 =
//!   source size, no resize.
//! - Cache key = (name, scale bits). Same character height -> same Arc share.
//! - `HARD_MAX_CACHE_BYTES` (config.rs) yahan construction pe clamp hota hai —
//!   §3/§11 ka last line of defence, Config::validate ke baad bhi.
//! - LRU: seq counter (koi linked list nahi, entries kam hote hain ~100–200).

use crate::config::HARD_MAX_CACHE_BYTES;
use image::imageops::FilterType;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

// ---------------- Errors ----------------

#[derive(Debug)]
pub enum AssetError {
    NotFound { name: String, path: PathBuf },
    Io { path: PathBuf, source: std::io::Error },
    Decode { path: PathBuf, source: image::ImageError },
    InvalidScale(f32),
    TooLarge { bytes: usize, max: usize },
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetError::NotFound { name, path } =>
                write!(f, "asset '{name}' not found at {}", path.display()),
            AssetError::Io { path, source } =>
                write!(f, "io error reading {}: {source}", path.display()),
            AssetError::Decode { path, source } =>
                write!(f, "png decode failed for {}: {source}", path.display()),
            AssetError::InvalidScale(s) =>
                write!(f, "invalid scale factor: {s}"),
            AssetError::TooLarge { bytes, max } =>
                write!(f, "scaled image {bytes} bytes exceeds cache budget {max}"),
        }
    }
}

impl std::error::Error for AssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AssetError::Io { source, .. } => Some(source),
            AssetError::Decode { source, .. } => Some(source),
            _ => None,
        }
    }
}

// ---------------- Cached image ----------------

#[derive(Debug)]
pub struct CachedImage {
    /// RGBA8, row-major, exactly `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Post-scale display size.
    pub width: u32,
    pub height: u32,
    /// Original (pre-scale) source size — pivot math ke liye.
    pub src_width: u32,
    pub src_height: u32,
}

impl CachedImage {
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    /// (sx, sy) to scale a source-space coordinate (pivot, attach) into
    /// display space. Caller multiplies the coordinate by these.
    pub fn pivot_scale(&self) -> (f32, f32) {
        (
            self.width as f32 / self.src_width as f32,
            self.height as f32 / self.src_height as f32,
        )
    }
}

// ---------------- Cache key ----------------

#[derive(Debug, Clone)]
struct CacheKey {
    name: String,
    scale_bits: u32,
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.scale_bits == other.scale_bits
    }
}

impl Eq for CacheKey {}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.name.hash(h);
        self.scale_bits.hash(h);
    }
}

// ---------------- LRU cache ----------------

pub struct AssetCache {
    entries: HashMap<CacheKey, (Arc<CachedImage>, u64)>,
    total_bytes: usize,
    max_bytes: usize,
    clock: u64,
}

impl AssetCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
            max_bytes: max_bytes.min(HARD_MAX_CACHE_BYTES),
            clock: 0,
        }
    }

    pub fn max_bytes(&self) -> usize { self.max_bytes }
    pub fn total_bytes(&self) -> usize { self.total_bytes }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    fn get(&mut self, key: &CacheKey) -> Option<Arc<CachedImage>> {
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.1 = clock;
        Some(entry.0.clone())
    }

    fn insert(&mut self, key: CacheKey, img: Arc<CachedImage>) -> Result<(), AssetError> {
        let bytes = img.rgba.len();
        if bytes > self.max_bytes {
            return Err(AssetError::TooLarge { bytes, max: self.max_bytes });
        }
        // Same key overwrite (shouldn't happen, but keep total honest).
        if let Some((old, _)) = self.entries.remove(&key) {
            self.total_bytes -= old.rgba.len();
        }
        // Evict LRU until new image fits.
        while self.total_bytes + bytes > self.max_bytes {
            if !self.evict_lru() {
                break; // defensive; unreachable since bytes <= max_bytes
            }
        }
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        self.entries.insert(key, (img, clock));
        self.total_bytes += bytes;
        Ok(())
    }

    fn evict_lru(&mut self) -> bool {
        let victim = self
            .entries
            .iter()
            .min_by_key(|(_, (_, seq))| *seq)
            .map(|(k, _)| k.clone());
        match victim {
            Some(k) => {
                if let Some((old, _)) = self.entries.remove(&k) {
                    self.total_bytes -= old.rgba.len();
                }
                true
            }
            None => false,
        }
    }
}

// ---------------- Loader ----------------

pub struct AssetLoader {
    root: PathBuf,
    cache: AssetCache,
}

impl AssetLoader {
    pub fn new(root: PathBuf, max_cache_bytes: usize) -> Self {
        Self { root, cache: AssetCache::new(max_cache_bytes) }
    }

    pub fn cache(&self) -> &AssetCache { &self.cache }

    /// Load `<root>/<name>.png`, scale to `scale`, cache as `Arc`, return.
    ///
    /// `scale` must be finite and > 0. Use `character.height` (§7). 1.0 = no resize.
    pub fn load(&mut self, name: &str, scale: f32) -> Result<Arc<CachedImage>, AssetError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(AssetError::InvalidScale(scale));
        }

        let key = CacheKey {
            name: name.to_string(),
            scale_bits: scale.to_bits(),
        };
        if let Some(img) = self.cache.get(&key) {
            return Ok(img);
        }

        let path = self.root.join(format!("{name}.png"));
        if !path.exists() {
            return Err(AssetError::NotFound { name: name.to_string(), path });
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| AssetError::Io { path: path.clone(), source: e })?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|e| AssetError::Decode { path: path.clone(), source: e })?;
        let rgba = decoded.to_rgba8();
        let (src_w, src_h) = rgba.dimensions();

        // Exactly-1.0 short-circuits (avoids a needless resize that could
        // subtly shift pixels — some resize implementations resample even at
        // identity).
        let (pixels, out_w, out_h) = if (scale - 1.0).abs() < 1e-6 {
            (rgba.into_raw(), src_w, src_h)
        } else {
            let tw = ((src_w as f32) * scale).round().max(1.0) as u32;
            let th = ((src_h as f32) * scale).round().max(1.0) as u32;
            let scaled = image::imageops::resize(&rgba, tw, th, FilterType::Triangle);
            (scaled.into_raw(), tw, th)
        };

        let cached = Arc::new(CachedImage {
            rgba: pixels,
            width: out_w,
            height: out_h,
            src_width: src_w,
            src_height: src_h,
        });

        self.cache.insert(key, cached.clone())?;
        Ok(cached)
    }
}

// ---------------- Tests ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TMP_ID: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("lkj-engine-assets-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tmp_root() -> PathBuf {
        let n = TMP_ID.fetch_add(1, Ordering::Relaxed);
        let root = tmp_dir().join(format!("root-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Runtime PNG (user point 3 — only in tests). Never used in production code.
    fn write_asset(root: &Path, name: &str, w: u32, h: u32, fill: Rgba<u8>) -> PathBuf {
        let mut path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        path.set_extension("png");
        let mut img = RgbaImage::new(w, h);
        for (_, _, p) in img.enumerate_pixels_mut() {
            *p = fill;
        }
        img.save(&path).unwrap();
        path
    }

    // ---- happy path ----

    #[test]
    fn load_returns_correct_dims_and_bytes() {
        let root = tmp_root();
        write_asset(&root, "body", 100, 200, Rgba([255, 0, 0, 255]));
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let img = ld.load("body", 1.0).unwrap();
        assert_eq!((img.width, img.height), (100, 200));
        assert_eq!((img.src_width, img.src_height), (100, 200));
        assert_eq!(img.rgba.len(), 100 * 200 * 4);
    }

    #[test]
    fn load_scales_half_and_pivot_scale_matches() {
        let root = tmp_root();
        write_asset(&root, "body", 100, 100, Rgba([0, 255, 0, 255]));
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let img = ld.load("body", 0.5).unwrap();
        assert_eq!((img.width, img.height), (50, 50));
        assert_eq!((img.src_width, img.src_height), (100, 100));
        let (sx, sy) = img.pivot_scale();
        assert!((sx - 0.5).abs() < 1e-6);
        assert!((sy - 0.5).abs() < 1e-6);
    }

    #[test]
    fn load_scale_one_does_not_resize() {
        let root = tmp_root();
        write_asset(&root, "body", 64, 32, Rgba([0, 0, 255, 255]));
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let img = ld.load("body", 1.0).unwrap();
        assert_eq!((img.width, img.height), (64, 32));
        let (sx, sy) = img.pivot_scale();
        assert_eq!(sx, 1.0);
        assert_eq!(sy, 1.0);
    }

    #[test]
    fn load_preserves_alpha_channel() {
        let root = tmp_root();
        let path = root.join("alpha.png");
        let mut img = RgbaImage::new(2, 2);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 0]));   // transparent
        img.put_pixel(1, 0, Rgba([0, 255, 0, 128])); // semi
        img.put_pixel(0, 1, Rgba([0, 0, 255, 255])); // opaque
        img.put_pixel(1, 1, Rgba([0, 0, 0, 255]));
        img.save(&path).unwrap();

        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let img = ld.load("alpha", 1.0).unwrap();
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 0]);
        assert_eq!(&img.rgba[4..8], &[0, 255, 0, 128]);
        assert_eq!(&img.rgba[8..12], &[0, 0, 255, 255]);
        assert_eq!(&img.rgba[12..16], &[0, 0, 0, 255]);
    }

    #[test]
    fn load_nested_name_reads_from_subdir() {
        let root = tmp_root();
        write_asset(&root, "oggy/part_6", 10, 10, Rgba([1, 2, 3, 255]));
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let img = ld.load("oggy/part_6", 1.0).unwrap();
        assert_eq!(img.width, 10);
    }

    // ---- caching ----

    #[test]
    fn second_load_returns_same_arc() {
        let root = tmp_root();
        write_asset(&root, "body", 10, 10, Rgba([1, 2, 3, 255]));
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let a = ld.load("body", 1.0).unwrap();
        let b = ld.load("body", 1.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "cache must return same Arc");
        assert_eq!(ld.cache().len(), 1);
    }

    #[test]
    fn different_scale_is_different_entry() {
        let root = tmp_root();
        write_asset(&root, "body", 100, 100, Rgba([0, 0, 0, 255]));
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let a = ld.load("body", 1.0).unwrap();
        let b = ld.load("body", 0.5).unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!((a.width, a.height), (100, 100));
        assert_eq!((b.width, b.height), (50, 50));
        assert_eq!(ld.cache().len(), 2);
    }

    // ---- errors ----

    #[test]
    fn missing_file_is_not_found() {
        let root = tmp_root();
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let err = ld.load("nope", 1.0).unwrap_err();
        assert!(matches!(err, AssetError::NotFound { .. }), "got {err:?}");
    }

    #[test]
    fn bad_png_is_decode_error() {
        let root = tmp_root();
        std::fs::write(root.join("bad.png"), b"not a png").unwrap();
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        let err = ld.load("bad", 1.0).unwrap_err();
        assert!(matches!(err, AssetError::Decode { .. }), "got {err:?}");
    }

    #[test]
    fn invalid_scales_rejected() {
        let root = tmp_root();
        let mut ld = AssetLoader::new(root, 10 * 1024 * 1024);
        for s in [0.0f32, -1.0, f32::INFINITY, f32::NAN] {
            let err = ld.load("body", s).unwrap_err();
            assert!(
                matches!(err, AssetError::InvalidScale(_)),
                "scale {s} got {err:?}"
            );
        }
    }

    #[test]
    fn too_large_image_rejected() {
        let root = tmp_root();
        write_asset(&root, "big", 100, 100, Rgba([0, 0, 0, 255])); // 40_000 bytes
        let mut ld = AssetLoader::new(root, 1000);
        let err = ld.load("big", 1.0).unwrap_err();
        assert!(matches!(err, AssetError::TooLarge { .. }), "got {err:?}");
    }

    // ---- LRU ----

    #[test]
    fn lru_evicts_oldest_when_over_budget() {
        let root = tmp_root();
        // 10x10 RGBA = 400 bytes each. Budget 1000 → fits 2.
        for n in ["a", "b", "c"] {
            write_asset(&root, n, 10, 10, Rgba([0, 0, 0, 255]));
        }
        let mut ld = AssetLoader::new(root, 1000);
        ld.load("a", 1.0).unwrap();
        ld.load("b", 1.0).unwrap();
        assert_eq!(ld.cache().len(), 2);
        assert_eq!(ld.cache().total_bytes(), 800);

        ld.load("c", 1.0).unwrap(); // evicts a (oldest)
        assert_eq!(ld.cache().len(), 2);
        assert_eq!(ld.cache().total_bytes(), 800);
    }

    #[test]
    fn lru_get_updates_recency() {
        let root = tmp_root();
        for n in ["a", "b", "c"] {
            write_asset(&root, n, 10, 10, Rgba([0, 0, 0, 255]));
        }
        let mut ld = AssetLoader::new(root, 1000);
        let a1 = ld.load("a", 1.0).unwrap();
        ld.load("b", 1.0).unwrap();
        // Touch a → a becomes MRU, b becomes LRU.
        let a2 = ld.load("a", 1.0).unwrap();
        assert!(Arc::ptr_eq(&a1, &a2));

        ld.load("c", 1.0).unwrap(); // evicts b, not a
        let a3 = ld.load("a", 1.0).unwrap();
        assert!(Arc::ptr_eq(&a1, &a3), "a must survive (recently used)");
    }

    #[test]
    fn total_bytes_never_exceeds_max() {
        let root = tmp_root();
        // 20x20 = 1600 bytes each, budget 5000 → max 3 fit.
        for n in ["a", "b", "c", "d", "e"] {
            write_asset(&root, n, 20, 20, Rgba([0, 0, 0, 255]));
        }
        let mut ld = AssetLoader::new(root, 5000);
        for n in ["a", "b", "c", "d", "e"] {
            let _ = ld.load(n, 1.0);
            assert!(
                ld.cache().total_bytes() <= ld.cache().max_bytes(),
                "budget exceeded after {n}: {} > {}",
                ld.cache().total_bytes(),
                ld.cache().max_bytes(),
            );
        }
        assert!(ld.cache().len() <= 3);
    }

    // ---- hard-limit defence ----

    #[test]
    fn cache_clamps_max_to_hard_limit() {
        // Even if caller passes usize::MAX, HARD_MAX_CACHE_BYTES wins.
        let c = AssetCache::new(usize::MAX);
        assert_eq!(c.max_bytes(), HARD_MAX_CACHE_BYTES);
    }
                                        }
