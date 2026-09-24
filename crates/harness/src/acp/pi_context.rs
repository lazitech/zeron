//! Opt-in Pi extension bridge for native context occupancy. pi-acp drops this
//! information; the extension uses Pi's own branch/compaction-aware API.
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use zeron_proto::ContextUsage;

const SOURCE: &str = include_str!("pi_context.mjs");
const MARKER: &str = "// Zeron-owned Pi context bridge.";
pub(super) const DIRECTORY_ENV: &str = "ZERON_PI_CONTEXT_DIR";

pub(super) struct Bridge {
    pub(super) directory: PathBuf,
    temporary: bool,
}

impl Bridge {
    pub(super) fn prepare(override_directory: Option<&Path>) -> std::io::Result<Self> {
        if let Some(directory) = override_directory {
            fs::create_dir_all(directory)?;
            return Ok(Self {
                directory: directory.to_owned(),
                temporary: false,
            });
        }
        let home = crate::executable::home_dir()
            .ok_or_else(|| std::io::Error::other("Pi context bridge requires a home directory"))?;
        let agent_dir = match std::env::var_os("PI_CODING_AGENT_DIR").filter(|s| !s.is_empty()) {
            Some(path) => {
                let path = PathBuf::from(path);
                if path == Path::new("~") {
                    home.clone()
                } else if let Ok(suffix) = path.strip_prefix("~/") {
                    home.join(suffix)
                } else {
                    path
                }
            }
            None => home.join(".pi/agent"),
        };
        install_extension(&agent_dir)?;
        let directory =
            std::env::temp_dir().join(format!("zeron-pi-context-{}", uuid::Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        Ok(Self {
            directory,
            temporary: true,
        })
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if self.temporary {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

fn install_extension(agent_dir: &Path) -> std::io::Result<()> {
    let directory = agent_dir.join("extensions");
    fs::create_dir_all(&directory)?;
    let destination = directory.join("zeron-context.js");
    match fs::read_to_string(&destination) {
        Ok(existing) if existing == SOURCE => return Ok(()),
        Ok(existing) if !existing.starts_with(MARKER) => {
            return Err(std::io::Error::other(
                "Pi context extension path contains a user-owned file",
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    // An ignored suffix prevents Pi discovering a half-written extension.
    let temporary = directory.join(format!(".zeron-context-{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temporary, SOURCE)?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

pub(super) fn read_snapshot(directory: &Path, session_id: &str) -> Option<ContextUsage> {
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Snapshot {
        session_id: String,
        tokens: Option<u64>,
        window: Option<u64>,
    }
    let raw = fs::read(directory.join(format!("{session_id}.json"))).ok()?;
    let snapshot: Snapshot = serde_json::from_slice(&raw).ok()?;
    (snapshot.session_id == session_id).then_some(ContextUsage {
        tokens: snapshot.tokens,
        window: snapshot.window.filter(|n| *n > 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_snapshot_is_scoped_and_preserves_unknown_after_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-1.json");
        fs::write(&path, r#"{"sessionId":"other","tokens":99,"window":1000}"#).unwrap();
        assert_eq!(read_snapshot(dir.path(), "session-1"), None);
        fs::write(
            &path,
            r#"{"sessionId":"session-1","tokens":null,"window":1000}"#,
        )
        .unwrap();
        assert_eq!(
            read_snapshot(dir.path(), "session-1"),
            Some(ContextUsage {
                tokens: None,
                window: Some(1000)
            })
        );
        assert_eq!(read_snapshot(dir.path(), "../other"), None);
    }

    #[test]
    fn extension_install_is_idempotent_and_preserves_user_files() {
        let dir = tempfile::tempdir().unwrap();
        install_extension(dir.path()).unwrap();
        install_extension(dir.path()).unwrap();
        let path = dir.path().join("extensions/zeron-context.js");
        assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
        fs::write(&path, "user file").unwrap();
        assert!(install_extension(dir.path()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "user file");
    }
}
