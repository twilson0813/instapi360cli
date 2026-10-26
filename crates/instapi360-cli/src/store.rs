//! File-backed [`SessionStore`] and config-dir helpers for the CLI.

use std::path::{Path, PathBuf};

use instapi360_cloud::{Result as CloudResult, Session, SessionStore};
use serde::{Deserialize, Serialize};

/// Small persisted CLI config (non-secret), e.g. the equipment code so it need
/// not be re-typed. Stored as JSON next to the session file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// `X-Equipment-Code` to send (a MAC-format id).
    pub equipment_code: Option<String>,
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default())
    }
}

/// Best-effort detect the host's primary MAC as a MAC-format equipment code
/// (`AA:BB:CC:DD:EE:FF`). Any stable value is accepted by the API; this avoids
/// hardcoding a real one. Tries Linux sysfs first, then `ifconfig` (macOS/BSD).
pub fn detect_equipment_code() -> Option<String> {
    detect_from_sysfs().or_else(detect_from_ifconfig)
}

/// Linux: read MACs from `/sys/class/net/*/address`.
fn detect_from_sysfs() -> Option<String> {
    let iface_dir = std::fs::read_dir("/sys/class/net").ok()?;
    let mut macs: Vec<String> = iface_dir
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != "lo")
        .filter_map(|e| std::fs::read_to_string(e.path().join("address")).ok())
        .map(|s| s.trim().to_uppercase())
        .filter(|m| is_usable_mac(m))
        .collect();
    macs.sort();
    macs.into_iter().next()
}

/// macOS / BSD: parse `ifconfig` output for `ether` lines.
fn detect_from_ifconfig() -> Option<String> {
    let out = std::process::Command::new("ifconfig").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut macs: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            l.strip_prefix("ether ")
                .map(|rest| rest.trim().to_uppercase())
        })
        .filter(|m| is_usable_mac(m))
        .collect();
    macs.sort();
    macs.into_iter().next()
}

/// A plausible, non-null MAC string `AA:BB:CC:DD:EE:FF`.
fn is_usable_mac(m: &str) -> bool {
    m.len() == 17 && m != "00:00:00:00:00:00"
}

/// Stores the session as JSON under the user's config dir.
pub struct FileSessionStore {
    path: PathBuf,
}

impl FileSessionStore {
    pub fn new(path: PathBuf) -> Self {
        FileSessionStore { path }
    }
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl SessionStore for FileSessionStore {
    fn load(&self) -> CloudResult<Option<Session>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let s: Session = serde_json::from_slice(&bytes)?;
                Ok(Some(s))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, session: &Session) -> CloudResult<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec_pretty(session)?;
        std::fs::write(&self.path, bytes)?;
        Ok(())
    }

    fn clear(&self) -> CloudResult<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
