//! Harness run artifacts and identifiers: the run manifest, session info,
//! run-directory layout, and UTC time / id / token helpers (no external date
//! dependency).

use super::{HARNESS_PROTOCOL_VERSION, RUN_MANIFEST_SCHEMA_VERSION};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Run artifacts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RunManifest {
    pub(crate) schema_version: u32,
    pub(crate) run_id: String,
    pub(crate) app_version: String,
    pub(crate) protocol_version: u32,
    pub(crate) sandboxed_storage: bool,
    pub(crate) started_at_epoch_ms: u64,
    pub(crate) started_at_utc: String,
    pub(crate) ended_at_epoch_ms: Option<u64>,
    pub(crate) ended_at_utc: Option<String>,
    /// `quit` (client-requested) — abnormal terminations leave this unset.
    pub(crate) stop_reason: Option<String>,
    pub(crate) artifacts: Vec<String>,
}

impl RunManifest {
    pub(crate) fn started(run_id: String, sandboxed_storage: bool) -> Self {
        let now_ms = epoch_ms_now();
        Self {
            schema_version: RUN_MANIFEST_SCHEMA_VERSION,
            run_id,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: HARNESS_PROTOCOL_VERSION,
            sandboxed_storage,
            started_at_epoch_ms: now_ms,
            started_at_utc: utc_string_from_epoch_ms(now_ms),
            ended_at_epoch_ms: None,
            ended_at_utc: None,
            stop_reason: None,
            artifacts: Vec::new(),
        }
    }

    pub(crate) fn finalize(&mut self, stop_reason: &str, artifacts: Vec<String>) {
        let now_ms = epoch_ms_now();
        self.ended_at_epoch_ms = Some(now_ms);
        self.ended_at_utc = Some(utc_string_from_epoch_ms(now_ms));
        self.stop_reason = Some(stop_reason.to_string());
        self.artifacts = artifacts;
    }
}

/// Contents of `session.json` — everything a client needs to connect.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SessionInfo {
    pub(crate) port: u16,
    pub(crate) token: String,
    pub(crate) pid: u32,
    pub(crate) run_id: String,
    pub(crate) protocol_version: u32,
}

pub(crate) fn harness_run_dir(base_root: &std::path::Path, run_id: &str) -> PathBuf {
    base_root.join("tmp").join("harness-runs").join(run_id)
}

pub(crate) struct SandboxStorageDirs {
    pub(crate) appdata: PathBuf,
    pub(crate) repo: PathBuf,
}

pub(crate) fn sandbox_storage_dirs(run_dir: &std::path::Path) -> SandboxStorageDirs {
    SandboxStorageDirs {
        appdata: run_dir.join("storage").join("appdata"),
        repo: run_dir.join("storage").join("repo"),
    }
}

pub(crate) fn write_manifest(
    run_dir: &std::path::Path,
    manifest: &RunManifest,
) -> Result<(), String> {
    let path = run_dir.join("manifest.json");
    let body = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("manifest serialization failed: {e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Time and identifier helpers (UTC; no external date dependency)
// ---------------------------------------------------------------------------

pub(crate) fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `YYYY-MM-DDTHH:MM:SSZ` from Unix epoch milliseconds (UTC civil calendar,
/// Howard Hinnant's `civil_from_days`).
pub(crate) fn utc_string_from_epoch_ms(epoch_ms: u64) -> String {
    let secs = (epoch_ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// `YYYYMMDD-HHMMSSZ-<pid>`, unique enough for one machine's run directories.
pub(crate) fn generate_run_id(epoch_ms: u64, pid: u32) -> String {
    let utc = utc_string_from_epoch_ms(epoch_ms);
    let compact: String = utc.chars().filter(|c| c.is_ascii_digit()).collect();
    format!("{}-{}Z-{pid}", &compact[..8], &compact[8..],)
}

/// 32-hex-char session token. Localhost-dev-tool strength: keeps other local
/// processes from driving the app by accident, not a cryptographic boundary.
pub(crate) fn generate_token(epoch_ms: u64, pid: u32) -> String {
    let seed = (epoch_ms << 20) ^ (u64::from(pid) << 1) ^ 0x9E37_79B9_7F4A_7C15;
    format!(
        "{:016x}{:016x}",
        splitmix64(seed),
        splitmix64(seed ^ 0x5851_F42D_4C95_7F2D)
    )
}

fn splitmix64(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::RUN_MANIFEST_SCHEMA_VERSION;

    #[test]
    fn utc_string_matches_known_instants() {
        assert_eq!(utc_string_from_epoch_ms(0), "1970-01-01T00:00:00Z");
        // 2026-01-01T00:00:00Z: 56 years, 14 leap days.
        assert_eq!(
            utc_string_from_epoch_ms(1_767_225_600_000),
            "2026-01-01T00:00:00Z"
        );
        // Leap-day handling: 2024-02-29T12:34:56Z
        // (1_709_164_800 = 2024-02-29T00:00:00Z; + 12h34m56s = 45_296s).
        assert_eq!(
            utc_string_from_epoch_ms((1_709_164_800 + 45_296) * 1000),
            "2024-02-29T12:34:56Z"
        );
    }

    #[test]
    fn run_id_is_compact_utc_plus_pid() {
        assert_eq!(
            generate_run_id(1_767_225_600_000, 4242),
            "20260101-000000Z-4242"
        );
    }

    #[test]
    fn tokens_are_32_hex_and_seed_sensitive() {
        let a = generate_token(1_000_000, 1);
        let b = generate_token(1_000_001, 1);
        let c = generate_token(1_000_000, 2);
        for token in [&a, &b, &c] {
            assert_eq!(token.len(), 32);
            assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn manifest_lifecycle_and_layout_helpers() {
        let mut manifest = RunManifest::started("run-1".to_string(), true);
        assert_eq!(manifest.schema_version, RUN_MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.app_version, env!("CARGO_PKG_VERSION"));
        assert!(manifest.ended_at_epoch_ms.is_none());
        assert!(manifest.stop_reason.is_none());

        manifest.finalize("quit", vec!["artifacts/0001-screenshot.png".to_string()]);
        assert_eq!(manifest.stop_reason.as_deref(), Some("quit"));
        assert!(manifest.ended_at_epoch_ms.unwrap() >= manifest.started_at_epoch_ms);
        assert_eq!(manifest.artifacts.len(), 1);

        let dir = harness_run_dir(std::path::Path::new("base"), "run-1");
        assert!(dir.ends_with(std::path::Path::new("tmp/harness-runs/run-1")));
        let storage = sandbox_storage_dirs(&dir);
        assert!(storage.appdata.ends_with(std::path::Path::new(
            "tmp/harness-runs/run-1/storage/appdata"
        )));
        assert!(storage
            .repo
            .ends_with(std::path::Path::new("tmp/harness-runs/run-1/storage/repo")));
    }

    #[test]
    fn manifest_written_to_run_dir() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = RunManifest::started("run-2".to_string(), false);
        write_manifest(dir.path(), &manifest).unwrap();
        let body = std::fs::read_to_string(dir.path().join("manifest.json")).unwrap();
        let back: RunManifest = serde_json::from_str(&body).unwrap();
        assert_eq!(back, manifest);
    }
}
