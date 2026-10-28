//! The verifiable seed payload: the progressive skill and contract docs ride
//! in the binary as bytes with digests, so a seed can be checked offline —
//! source-relative links alone are not a payload.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

pub struct PayloadFile {
    /// Path relative to the payload root (the installed layout).
    pub install_path: &'static str,
    /// Where the bytes come from in the repo SSOT (provenance only).
    pub repo_path: &'static str,
    pub bytes: &'static str,
}

/// One place to extend; the manifest and every verifier derive from this.
pub const FILES: &[PayloadFile] = &[
    PayloadFile {
        install_path: "commands.json",
        repo_path: "tools/swarmharness/introspection/commands.json",
        bytes: crate::ir::EMBEDDED_IR,
    },
    PayloadFile {
        install_path: "skills/yggterm/SKILL.md",
        repo_path: ".agents/skills/yggterm/SKILL.md",
        bytes: include_str!("../../../.agents/skills/yggterm/SKILL.md"),
    },
    PayloadFile {
        install_path: "docs/yggterm-4-system.md",
        repo_path: "docs/yggterm-4-system.md",
        bytes: include_str!("../../../docs/yggterm-4-system.md"),
    },
    PayloadFile {
        install_path: "docs/ygg4-cli-contract.md",
        repo_path: "docs/ygg4-cli-contract.md",
        bytes: include_str!("../../../docs/ygg4-cli-contract.md"),
    },
    PayloadFile {
        install_path: "docs/swarm-introspection.md",
        repo_path: "docs/swarm-introspection.md",
        bytes: include_str!("../../../docs/swarm-introspection.md"),
    },
];

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Serialize, Clone)]
pub struct ManifestEntry {
    pub path: String,
    pub source: String,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub contract: String,
    pub payload: Vec<ManifestEntry>,
}

pub fn manifest() -> Manifest {
    Manifest {
        schema_version: 1,
        contract: crate::ir::CONTRACT_NAME.to_string(),
        payload: FILES
            .iter()
            .map(|f| ManifestEntry {
                path: f.install_path.to_string(),
                source: f.repo_path.to_string(),
                sha256: sha256_hex(f.bytes.as_bytes()),
                bytes: f.bytes.len(),
            })
            .collect(),
    }
}

#[derive(Serialize)]
pub struct VerifyRow {
    pub path: String,
    pub status: String, // verified | missing | mismatch | unreadable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found_sha256: Option<String>,
}

#[derive(Serialize)]
pub struct VerifyReport {
    pub schema_version: u32,
    pub dir: String,
    pub rows: Vec<VerifyRow>,
    pub verified: usize,
    pub failed: usize,
}

#[derive(Debug)]
pub enum PayloadOutcome {
    Done(VerifyReport),
    /// The payload directory itself is absent: exit 3.
    Unavailable(String),
    /// An installer-direction refusal (one-owner law, bad target): exit 3.
    Refused(String),
    IoError(String),
}

fn file_digest(path: &Path) -> Result<String, std::io::Error> {
    let bytes = fs::read(path)?;
    Ok(sha256_hex(&bytes))
}

pub fn verify(dir: &Path) -> PayloadOutcome {
    if !dir.is_dir() {
        return PayloadOutcome::Unavailable(format!(
            "ygg: payload directory unavailable: {}; install a payload or pass --dir",
            dir.display()
        ));
    }
    let mut rows = Vec::new();
    let (mut ok, mut bad) = (0usize, 0usize);
    for file in FILES {
        let target = dir.join(file.install_path);
        if !target.exists() {
            rows.push(VerifyRow {
                path: file.install_path.into(),
                status: "missing".into(),
                found_sha256: None,
            });
            bad += 1;
            continue;
        }
        match file_digest(&target) {
            Ok(found) => {
                let verified = found == sha256_hex(file.bytes.as_bytes());
                rows.push(VerifyRow {
                    path: file.install_path.into(),
                    status: if verified { "verified".into() } else { "mismatch".into() },
                    found_sha256: Some(found),
                });
                if verified {
                    ok += 1;
                } else {
                    bad += 1;
                }
            }
            Err(e) => {
                rows.push(VerifyRow {
                    path: file.install_path.into(),
                    status: "unreadable".into(),
                    found_sha256: None,
                });
                bad += 1;
                let _ = e;
            }
        }
    }
    PayloadOutcome::Done(VerifyReport {
        schema_version: 1,
        dir: dir.display().to_string(),
        rows,
        verified: ok,
        failed: bad,
    })
}

pub fn install(dir: &Path) -> PayloadOutcome {
    // One-owner law (docs/spec-ygg-verb-dispersal.md): only ygg-disperse.py
    // writes the dispersed replica tree. The payload installer stages
    // elsewhere and says so.
    if let Some(dispersed) = crate::ir::payload_dir() {
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        if canon(dir) == canon(&dispersed) {
            return PayloadOutcome::Refused(format!(
                "ygg: refusing to write the dispersed payload tree ({}); that subtree is owned by ygg-disperse.py install",
                dispersed.display()
            ));
        }
    }
    for file in FILES {
        let target = dir.join(file.install_path);
        if let Some(parent) = target.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return PayloadOutcome::IoError(format!(
                    "ygg: cannot create payload directory {}: {e}",
                    parent.display()
                ));
            }
        }
        // Idempotent: identical bytes stay untouched; a differing file is
        // replaced through a same-directory rename, not written in place.
        if target.is_file() {
            if let Ok(found) = file_digest(&target) {
                if found == sha256_hex(file.bytes.as_bytes()) {
                    continue;
                }
            }
        }
        let temp = target.with_extension("ygg-payload-tmp");
        if let Err(e) = fs::write(&temp, file.bytes) {
            return PayloadOutcome::IoError(format!(
                "ygg: cannot stage {}: {e}",
                temp.display()
            ));
        }
        if let Err(e) = fs::rename(&temp, &target) {
            let _ = fs::remove_file(&temp);
            return PayloadOutcome::IoError(format!(
                "ygg: cannot install {}: {e}",
                target.display()
            ));
        }
    }
    verify(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("ygg-cli-payload-test-{}-{}", std::process::id(), label));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn manifest_digests_match_the_embedded_bytes() {
        let manifest = manifest();
        assert_eq!(manifest.payload.len(), FILES.len());
        for entry in &manifest.payload {
            assert_eq!(entry.sha256.len(), 64);
            let file = FILES.iter().find(|f| f.install_path == entry.path).unwrap();
            assert_eq!(entry.bytes, file.bytes.len());
        }
    }

    #[test]
    fn install_then_verify_round_trips() {
        let dir = scratch_dir("roundtrip");
        let outcome = install(&dir);
        let report = match outcome {
            PayloadOutcome::Done(report) => report,
            other => panic!("expected done, got {other:?}"),
        };
        assert_eq!(report.failed, 0);
        let verify_outcome = verify(&dir);
        match verify_outcome {
            PayloadOutcome::Done(report) => {
                assert_eq!(report.failed, 0);
                assert_eq!(report.verified, FILES.len());
            }
            other => panic!("expected done, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let dir = scratch_dir("tamper");
        assert!(matches!(install(&dir), PayloadOutcome::Done(_)));
        let target = dir.join(FILES[0].install_path);
        fs::write(&target, "tampered bytes").expect("tamper");
        match verify(&dir) {
            PayloadOutcome::Done(report) => {
                assert_eq!(report.failed, 1);
                assert_eq!(report.rows[0].status, "mismatch");
            }
            other => panic!("expected done, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn absent_directory_is_unavailable_not_empty() {
        let dir = std::env::temp_dir().join("ygg-cli-payload-test-absent-does-not-exist");
        let _ = fs::remove_dir_all(&dir);
        assert!(matches!(verify(&dir), PayloadOutcome::Unavailable(_)));
    }
}
