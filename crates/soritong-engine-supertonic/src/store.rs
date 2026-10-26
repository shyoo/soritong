//! Download and verification of the Supertonic 3 model files.
//!
//! Files come from Hugging Face at a pinned commit, never `main`, and each is checked against a
//! sha256 recorded here. So an upstream change, or a tampered mirror, can only fail the
//! download. It cannot swap the model under a cached settings fingerprint.

use sha2::{Digest, Sha256};
use soritong_core::{Result, TtsError};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

pub const HF_REPO: &str = "supertone-oss-archive/supertonic-3";
/// The archived snapshot upstream's own quick start pins.
pub const REVISION: &str = "aafc6e32416a594460b32413efc49d7fe4ce6d46";

/// One file of the model: path inside the repo (and inside the store), size and sha256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    pub path: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

const fn f(path: &'static str, size: u64, sha256: &'static str) -> ModelFile {
    ModelFile { path, size, sha256 }
}

/// Everything [`crate::SupertonicEngine`] needs. `LICENSE` (OpenRAIL-M) is fetched too, because
/// the licence must travel with the weights.
pub const FILES: &[ModelFile] = &[
    f(
        "onnx/duration_predictor.onnx",
        3_700_147,
        "c3eb91414d5ff8a7a239b7fe9e34e7e2bf8a8140d8375ffb14718b1c639325db",
    ),
    f(
        "onnx/text_encoder.onnx",
        36_416_150,
        "c7befd5ea8c3119769e8a6c1486c4edc6a3bc8365c67621c881bbb774b9902ff",
    ),
    f(
        "onnx/vector_estimator.onnx",
        256_534_781,
        "883ac868ea0275ef0e991524dc64f16b3c0376efd7c320af6b53f5b780d7c61c",
    ),
    f(
        "onnx/vocoder.onnx",
        101_424_195,
        "085de76dd8e8d5836d6ca66826601f615939218f90e519f70ee8a36ed2a4c4ba",
    ),
    f(
        "onnx/tts.json",
        8_253,
        "42078d3aef1cd43ab43021f3c54f47d2d75ceb4e75f627f118890128b06a0d09",
    ),
    f(
        "onnx/unicode_indexer.json",
        277_676,
        "9bf7346e43883a81f8645c81224f786d43c5b57f3641f6e7671a7d6c493cb24f",
    ),
    f(
        "voice_styles/F1.json",
        292_046,
        "bbdec6ee00231c2c742ad05483df5334cab3b52fda3ba38e6a07059c4563dbc2",
    ),
    f(
        "voice_styles/F2.json",
        292_423,
        "7c722c6a72707b1a77f035d67f0d1351ba187738e06f7683e8c72b1df3477fc6",
    ),
    f(
        "voice_styles/F3.json",
        290_794,
        "12f6ef2573baa2defa1128069cb59f203e3ab67c92af77b42df8a0e3a2f7c6ab",
    ),
    f(
        "voice_styles/F4.json",
        291_808,
        "c2fa764c1225a76dfc3e2c73e8aa4f70d9ee48793860eb34c295fff01c2e032b",
    ),
    f(
        "voice_styles/F5.json",
        291_479,
        "45966e73316415626cf41a7d1c6f3b4c70dbc1ba2bee5c1978ef0ce33244fc8d",
    ),
    f(
        "voice_styles/M1.json",
        291_748,
        "e35604687f5d23694b8e91593a93eec0e4eca6c0b02bb8ed69139ab2ea6b0a5b",
    ),
    f(
        "voice_styles/M2.json",
        292_055,
        "b76cbf62bac707c710cf0ae5aba5e31eea1a6339a9734bfae33ab98499534a50",
    ),
    f(
        "voice_styles/M3.json",
        290_198,
        "ea1ac35ccb91b0d7ecad533a2fbd0eec10c91513d8951e3b25fbba99954e159b",
    ),
    f(
        "voice_styles/M4.json",
        291_522,
        "ca8eefad4fcd989c9379032ff3e50738adc547eeb5e221b82593a6d7b3bac303",
    ),
    f(
        "voice_styles/M5.json",
        291_469,
        "dd22b92740314321f8ae11c5e87f8dd60d060f15dd3a632b5adf77f471f77af2",
    ),
    f(
        "LICENSE",
        15_007,
        "0d944a9110fed9a9602d60e0423a272903e7bd21ab060490774efc77c2275e9f",
    ),
];

/// Bytes [`ModelStore::ensure`] downloads on a fresh machine (≈ 401 MB), for a "download model" button.
pub const TOTAL_BYTES: u64 = {
    let mut sum = 0;
    let mut i = 0;
    while i < FILES.len() {
        sum += FILES[i].size;
        i += 1;
    }
    sum
};

/// The voice styles shipped in the snapshot, named as upstream names them.
pub const VOICES: &[&str] = &["F1", "F2", "F3", "F4", "F5", "M1", "M2", "M3", "M4", "M5"];

/// Where the verified files are. Build it with [`ModelStore::ensure`], or [`ModelPaths::new`]
/// for files you placed yourself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPaths {
    pub root: PathBuf,
}

impl ModelPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn onnx(&self, name: &str) -> PathBuf {
        self.root.join("onnx").join(name)
    }
    pub fn voice_style(&self, voice: &str) -> PathBuf {
        self.root.join("voice_styles").join(format!("{voice}.json"))
    }
}

#[derive(Debug, Clone)]
pub struct ModelStore {
    dir: PathBuf,
}

impl Default for ModelStore {
    /// `%LOCALAPPDATA%\soritong\models\supertonic-3` on Windows.
    fn default() -> Self {
        if let Some(p) = std::env::var_os("SORITONG_SUPERTONIC_MODEL_DIR") {
            return Self::new(PathBuf::from(p));
        }
        let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
        Self::new(base.join("soritong").join("models").join("supertonic-3"))
    }
}

impl ModelStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn paths(&self) -> ModelPaths {
        ModelPaths::new(&self.dir)
    }

    /// True when every file is present and was verified. Cheap: it reads the small `.sha256`
    /// markers written after verification and never re-hashes the 400 MB of weights.
    pub fn is_ready(&self) -> bool {
        FILES.iter().all(|f| self.is_verified(f))
    }

    /// Downloads whatever is missing or does not verify, then returns the paths.
    /// `progress(done, total)` counts bytes over the whole model, including files already present.
    /// Blocking; run it on a worker thread.
    pub fn ensure(&self, progress: &mut dyn FnMut(u64, u64)) -> Result<ModelPaths> {
        let mut done = 0;
        for file in FILES {
            let path = self.dir.join(file.path);
            if !self.is_verified(file) {
                if path.exists() && sha256_file(&path)? == file.sha256 {
                    self.mark_verified(file)?;
                } else {
                    let base = done;
                    let mut report = |n: u64| progress(base + n, TOTAL_BYTES);
                    download(&url(file.path), &path, file, &mut report)?;
                    self.mark_verified(file)?;
                }
            }
            done += file.size;
            progress(done, TOTAL_BYTES);
        }
        Ok(self.paths())
    }

    fn marker(&self, file: &ModelFile) -> PathBuf {
        self.dir.join(format!("{}.sha256", file.path))
    }

    fn is_verified(&self, file: &ModelFile) -> bool {
        let path = self.dir.join(file.path);
        std::fs::metadata(&path).is_ok_and(|m| m.len() == file.size)
            && std::fs::read_to_string(self.marker(file)).is_ok_and(|s| s.trim() == file.sha256)
    }

    fn mark_verified(&self, file: &ModelFile) -> Result<()> {
        std::fs::write(self.marker(file), file.sha256)?;
        Ok(())
    }
}

pub fn url(path: &str) -> String {
    format!("https://huggingface.co/{HF_REPO}/resolve/{REVISION}/{path}")
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut r = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Streams `src` into `dest.part`, hashing as it goes, and renames only when size and hash match.
pub fn copy_verified(
    src: &mut dyn Read,
    dest: &Path,
    file: &ModelFile,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
    ));
    let mut out = File::create(&part)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n_total = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        n_total += n as u64;
        progress(n_total);
    }
    out.flush()?;
    drop(out);
    let got = hex(&h.finalize());
    if n_total != file.size || got != file.sha256 {
        std::fs::remove_file(&part).ok();
        return Err(TtsError::Engine(format!(
            "{}: expected {} bytes sha256 {}, got {n_total} bytes sha256 {got}",
            file.path, file.size, file.sha256
        )));
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

fn download(url: &str, dest: &Path, file: &ModelFile, progress: &mut dyn FnMut(u64)) -> Result<()> {
    log::info!("downloading {url}");
    let resp = ureq::get(url)
        .call()
        .map_err(|e| TtsError::Network(format!("{url}: {e}")))?;
    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    copy_verified(&mut reader, dest, file, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("soritong-st-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        d
    }

    // sha256("hello")
    const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn verified_copy_lands_only_on_matching_hash() {
        let dir = temp("copy");
        let dest = dir.join("onnx").join("x.onnx");
        let good = ModelFile {
            path: "onnx/x.onnx",
            size: 5,
            sha256: HELLO,
        };
        let mut seen = 0;
        copy_verified(&mut &b"hello"[..], &dest, &good, &mut |n| seen = n).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert_eq!(seen, 5);
        assert_eq!(sha256_file(&dest).unwrap(), HELLO);

        let other = dir.join("onnx").join("y.onnx");
        let err = copy_verified(&mut &b"hellO"[..], &other, &good, &mut |_| {});
        assert!(err.is_err());
        assert!(!other.exists(), "a bad download never takes the real name");
        assert!(!dir.join("onnx").join("y.onnx.part").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn store_layout_and_readiness() {
        let dir = temp("store");
        let store = ModelStore::new(&dir);
        assert!(!store.is_ready());
        let p = store.paths();
        assert_eq!(
            p.onnx("vocoder.onnx"),
            dir.join("onnx").join("vocoder.onnx")
        );
        assert_eq!(
            p.voice_style("F1"),
            dir.join("voice_styles").join("F1.json")
        );
        assert!(
            ModelStore::default()
                .dir()
                .ends_with(Path::new("soritong").join("models").join("supertonic-3"))
        );

        // A file of the right size without a marker is not trusted; a marker makes it ready.
        let file = &FILES[0];
        let path = dir.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![0u8; file.size as usize]).unwrap();
        assert!(!store.is_verified(file));
        store.mark_verified(file).unwrap();
        assert!(store.is_verified(file));
        // Truncation is noticed without hashing.
        std::fs::write(&path, b"short").unwrap();
        assert!(!store.is_verified(file));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn manifest_is_consistent() {
        const { assert!(TOTAL_BYTES > 390_000_000 && TOTAL_BYTES < 410_000_000) };
        for f in FILES {
            assert_eq!(f.sha256.len(), 64, "{}", f.path);
            assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        }
        for v in VOICES {
            assert!(
                FILES
                    .iter()
                    .any(|f| f.path == format!("voice_styles/{v}.json"))
            );
        }
        assert!(url("onnx/tts.json").contains(REVISION));
        assert!(!url("onnx/tts.json").contains("/main/"));
    }
}
