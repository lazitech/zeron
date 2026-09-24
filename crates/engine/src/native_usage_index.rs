//! Read-only index of native coding-agent histories.
//!
//! Wake owns the parsers for the native history formats.  This module only
//! keeps the small amount of metadata needed by the Statistics page: session
//! identity, project/model metadata, prompt timestamps, token totals, and the
//! source file fingerprint.  It never copies transcript text into Zeron's
//! index database.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wake_core::adapters::{AgentAdapter, create_adapters};
use wake_core::models::{Role, SessionFileRef};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS native_sessions (
    key                    TEXT PRIMARY KEY,
    agent                  TEXT NOT NULL,
    native_id              TEXT NOT NULL,
    source                 TEXT NOT NULL,
    source_path            TEXT NOT NULL,
    source_fingerprint     TEXT NOT NULL,
    source_mtime_ms        INTEGER NOT NULL,
    source_size_bytes      INTEGER NOT NULL,
    project_path           TEXT,
    project_name           TEXT,
    model                  TEXT,
    archived               INTEGER NOT NULL DEFAULT 0,
    created_at_ms          INTEGER NOT NULL DEFAULT 0,
    updated_at_ms          INTEGER NOT NULL DEFAULT 0,
    prompts                INTEGER NOT NULL DEFAULT 0,
    prompt_timestamps_ms   TEXT NOT NULL DEFAULT '[]',
    tokens_used            INTEGER,
    message_count          INTEGER NOT NULL DEFAULT 0,
    unknown_line_count     INTEGER NOT NULL DEFAULT 0,
    token_usage_events     TEXT NOT NULL DEFAULT '[]'
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_native_sessions_source_path
    ON native_sessions(source_path);
CREATE INDEX IF NOT EXISTS idx_native_sessions_agent
    ON native_sessions(agent, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_native_sessions_project
    ON native_sessions(project_path);
"#;

/// One native session with all fields needed to aggregate statistics.
///
/// The key is stable across refreshes and includes the agent, native id, and
/// creation timestamp used by Wake to distinguish reused ids.
/// `prompt_timestamps_ms` intentionally omits prompts whose source record has
/// no timestamp; those prompts are still counted by `prompts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeSessionSummary {
    pub key: String,
    pub agent: String,
    pub native_id: String,
    pub source: String,
    pub source_path: String,
    pub source_fingerprint: String,
    pub source_mtime_ms: i64,
    pub source_size_bytes: u64,
    pub project_path: Option<String>,
    pub project_name: Option<String>,
    pub model: Option<String>,
    pub archived: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub prompts: u64,
    pub prompt_timestamps_ms: Vec<i64>,
    pub tokens_used: Option<u64>,
    pub token_usage_events: Vec<crate::native_token_usage::TokenUsageEvent>,
    pub message_count: u64,
    pub unknown_line_count: u32,
}

/// Per adapter scan counters.  An adapter error does not abort the other
/// adapters; its error is retained here so the caller can display incomplete
/// coverage instead of presenting a plausible but incomplete total.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NativeAdapterCoverage {
    pub source: String,
    pub discovered_files: u64,
    pub indexed_sessions: u64,
    pub reused_sessions: u64,
    pub parsed_sessions: u64,
    pub failed_files: u64,
    pub error: Option<String>,
}

/// Coverage information for one refresh.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NativeUsageCoverage {
    pub adapters: Vec<NativeAdapterCoverage>,
    pub discovered_files: u64,
    pub indexed_sessions: u64,
    pub reused_sessions: u64,
    pub parsed_sessions: u64,
    pub failed_files: u64,
    pub failed_adapters: u64,
    pub unknown_lines: u64,
    pub partial: bool,
}

/// Persisted native session summaries plus the coverage of the latest refresh.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NativeUsageSnapshot {
    pub sessions: Vec<NativeSessionSummary>,
    pub coverage: NativeUsageCoverage,
}

/// SQLite-backed metadata index for native agent histories.
#[derive(Debug, Clone)]
pub struct NativeUsageIndex {
    path: PathBuf,
    refresh_gate: Arc<Mutex<()>>,
}

#[derive(Debug, Clone)]
struct ExistingSummary {
    summary: NativeSessionSummary,
}

impl NativeUsageIndex {
    /// Open (and initialize) the Zeron-owned metadata index.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("create native usage index directory {}", parent.display())
            })?;
        }
        let index = Self {
            path,
            refresh_gate: Arc::new(Mutex::new(())),
        };
        let connection = index.connection()?;
        connection
            .execute_batch(SCHEMA)
            .context("initialize native usage index schema")?;
        // The index is local application state and may have been created by an
        // earlier development build. Migrate it without masking unrelated SQL
        // failures, while keeping the new-install path idempotent.
        let archived_column_exists = {
            let mut statement = connection
                .prepare("PRAGMA table_info(native_sessions)")
                .context("inspect native usage index schema")?;
            let names = statement.query_map([], |row| row.get::<_, String>(1))?;
            names
                .collect::<std::result::Result<Vec<_>, _>>()?
                .iter()
                .any(|name| name == "archived")
        };
        if !archived_column_exists {
            connection
                .execute(
                    "ALTER TABLE native_sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .context("migrate native usage index archived column")?;
        }
        let project_name_column_exists = {
            let mut statement = connection
                .prepare("PRAGMA table_info(native_sessions)")
                .context("inspect native usage index project name")?;
            let names = statement.query_map([], |row| row.get::<_, String>(1))?;
            names
                .collect::<std::result::Result<Vec<_>, _>>()?
                .iter()
                .any(|name| name == "project_name")
        };
        if !project_name_column_exists {
            connection
                .execute(
                    "ALTER TABLE native_sessions ADD COLUMN project_name TEXT",
                    [],
                )
                .context("migrate native usage index project name column")?;
        }
        let token_usage_events_column_exists = {
            let mut statement = connection
                .prepare("PRAGMA table_info(native_sessions)")
                .context("inspect native token usage index schema")?;
            let names = statement.query_map([], |row| row.get::<_, String>(1))?;
            names
                .collect::<std::result::Result<Vec<_>, _>>()?
                .iter()
                .any(|name| name == "token_usage_events")
        };
        if !token_usage_events_column_exists {
            connection
                .execute(
                    "ALTER TABLE native_sessions ADD COLUMN token_usage_events TEXT NOT NULL DEFAULT '[]'",
                    [],
                )
                .context("migrate native token usage index events")?;
        }
        Ok(index)
    }

    /// Scan every local Wake adapter and return the metadata snapshot.
    ///
    /// Only files whose `(path, mtime, size)` fingerprint changed are parsed.
    /// Native source files are opened through Wake's read-only adapters; the
    /// only writes made by this method are to `self.path`.
    pub fn refresh_and_load(&self) -> Result<NativeUsageSnapshot> {
        // The RPC layer may issue overlapping refreshes while a previous scan
        // is still running.  Keep one NativeUsageIndex instance's read/scan/
        // rewrite sequence atomic so a slower scan cannot overwrite a newer
        // snapshot.  Separate instances are still protected by SQLite's busy
        // timeout; callers should share the opened index when possible.
        let _refresh_guard = self
            .refresh_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let connection = self.connection()?;
        let existing = load_existing(&connection)?;
        let adapters = create_adapters();
        let mut candidates = BTreeMap::<String, NativeSessionSummary>::new();
        let mut coverage = NativeUsageCoverage::default();

        for adapter in adapters {
            let source = adapter.agent().as_str().to_owned();
            let roots = adapter.data_roots();
            let mut adapter_coverage = NativeAdapterCoverage {
                source: source.clone(),
                ..NativeAdapterCoverage::default()
            };

            let refs = match check_adapter_roots(&roots).and_then(|()| adapter.list_session_files())
            {
                Ok(refs) if refs.is_empty() => {
                    let cached = cached_summaries_for_roots(&existing, &source, &roots, true);
                    if cached.is_empty() {
                        refs
                    } else {
                        tracing::warn!(
                            adapter = %source,
                            "native history adapter returned no sessions while indexed source files remain; retaining cached summaries"
                        );
                        adapter_coverage.error = Some(
                            "adapter returned no sessions while indexed source files remain".into(),
                        );
                        coverage.failed_adapters = coverage.failed_adapters.saturating_add(1);
                        coverage.partial = true;
                        retain_cached_summaries(&mut candidates, &mut coverage, cached);
                        coverage.adapters.push(adapter_coverage);
                        continue;
                    }
                }
                Ok(refs) => refs,
                Err(error) => {
                    adapter_coverage.error = Some(error.to_string());
                    coverage.failed_adapters = coverage.failed_adapters.saturating_add(1);
                    coverage.partial = true;
                    // Keep the adapter's last good summaries when enumeration
                    // fails transiently. Otherwise rewriting the index below
                    // would turn an unavailable source into zero sessions.
                    let cached = cached_summaries_for_roots(&existing, &source, &roots, false);
                    retain_cached_summaries(&mut candidates, &mut coverage, cached);
                    coverage.adapters.push(adapter_coverage);
                    continue;
                }
            };

            adapter_coverage.discovered_files = refs.len() as u64;
            coverage.discovered_files = coverage
                .discovered_files
                .saturating_add(adapter_coverage.discovered_files);
            let project_path_updates = adapter.project_path_updates(&refs);

            for file_ref in refs {
                let fingerprint = source_fingerprint(&file_ref);
                let reusable = existing
                    .get(&file_ref.file_path)
                    .filter(|stored| stored.summary.source_fingerprint == fingerprint)
                    .map(|stored| stored.summary.clone());

                let mut summary = if let Some(summary) = reusable {
                    adapter_coverage.reused_sessions =
                        adapter_coverage.reused_sessions.saturating_add(1);
                    coverage.reused_sessions = coverage.reused_sessions.saturating_add(1);
                    summary
                } else {
                    match parse_summary(adapter.as_ref(), &file_ref, &fingerprint) {
                        Ok(summary) => {
                            adapter_coverage.parsed_sessions =
                                adapter_coverage.parsed_sessions.saturating_add(1);
                            coverage.parsed_sessions = coverage.parsed_sessions.saturating_add(1);
                            summary
                        }
                        Err(error) => {
                            adapter_coverage.failed_files =
                                adapter_coverage.failed_files.saturating_add(1);
                            coverage.failed_files = coverage.failed_files.saturating_add(1);
                            coverage.partial = true;
                            tracing::warn!(
                                adapter = %source,
                                path = %file_ref.file_path,
                                %error,
                                "native usage session parse failed"
                            );
                            // Keep the last good metadata for a changed file.  A
                            // transiently torn JSONL tail must not make historical
                            // usage disappear from the dashboard.
                            if let Some(stored) = existing.get(&file_ref.file_path) {
                                stored.summary.clone()
                            } else {
                                continue;
                            }
                        }
                    }
                };

                // Count the summary that actually enters the index, including
                // a cached fallback after a changed native file fails to parse.
                coverage.unknown_lines = coverage
                    .unknown_lines
                    .saturating_add(summary.unknown_line_count as u64);
                coverage.partial |= summary.unknown_line_count > 0;

                if let Some(project_path) = project_path_updates
                    .get(&file_ref.file_path)
                    .filter(|path| !path.trim().is_empty())
                {
                    summary.project_path = Some(project_path.clone());
                    summary.project_name = Some(
                        std::path::Path::new(project_path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| "Unknown project".into()),
                    );
                }
                insert_candidate(&mut candidates, summary);
            }

            coverage.adapters.push(adapter_coverage);
        }

        for adapter in &mut coverage.adapters {
            adapter.indexed_sessions = candidates
                .values()
                .filter(|summary| summary.source == adapter.source)
                .count() as u64;
            if adapter.failed_files > 0 || adapter.error.is_some() {
                coverage.partial = true;
            }
        }
        coverage.indexed_sessions = candidates.len() as u64;

        write_sessions(&connection, candidates.values())?;
        let mut sessions: Vec<_> = candidates.into_values().collect();
        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.key.cmp(&right.key))
        });

        Ok(NativeUsageSnapshot { sessions, coverage })
    }
}

impl NativeUsageIndex {
    fn connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)
            .with_context(|| format!("open native usage index {}", self.path.display()))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(3))
            .context("configure native usage index busy timeout")?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("enable native usage index WAL")?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .context("configure native usage index durability")?;
        Ok(connection)
    }
}

fn parse_summary(
    adapter: &dyn AgentAdapter,
    file_ref: &SessionFileRef,
    fingerprint: &str,
) -> Result<NativeSessionSummary> {
    let parsed = adapter
        .parse_session(file_ref)
        .with_context(|| format!("parse native session {}", file_ref.file_path))?;
    let token_usage_events = crate::native_token_usage::extract(file_ref).unwrap_or_else(|error| {
        tracing::warn!(
            agent = %file_ref.agent.as_str(),
            path = %file_ref.file_path,
            %error,
            "native token usage events could not be parsed"
        );
        Vec::new()
    });
    let meta = parsed.meta;
    let native_id = if meta.id.trim().is_empty() {
        file_ref.native_id.clone()
    } else {
        meta.id.clone()
    };
    let agent = adapter.agent().as_str().to_owned();
    let created_at_ms = meta.created_at;
    // Wake's canonical-session key includes the creation timestamp so a
    // provider that reuses a native id for a later session does not collapse
    // two distinct conversations.
    let key = if created_at_ms > 0 {
        format!("{agent}:{native_id}:{created_at_ms}")
    } else {
        format!("{agent}:{native_id}:unknown-time:{}", file_ref.file_path)
    };

    let mut prompt_timestamps_ms = parsed
        .units
        .iter()
        .filter(|unit| unit.sidechain_id.is_none() && unit.role == Role::User)
        .filter_map(|unit| unit.timestamp)
        .collect::<Vec<_>>();
    prompt_timestamps_ms.sort_unstable();

    let prompts = parsed
        .units
        .iter()
        .filter(|unit| unit.sidechain_id.is_none() && unit.role == Role::User)
        .count() as u64;
    let project_path = non_empty(meta.project_path);
    let project_name = non_empty(meta.project_name).or_else(|| {
        project_path.as_deref().map(|path| {
            std::path::Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Unknown project".into())
        })
    });

    Ok(NativeSessionSummary {
        key,
        agent: agent.clone(),
        native_id,
        source: agent,
        source_path: file_ref.file_path.clone(),
        source_fingerprint: fingerprint.to_owned(),
        source_mtime_ms: file_ref.mtime_ms,
        source_size_bytes: file_ref.size.max(0) as u64,
        project_path,
        project_name,
        model: meta.model.and_then(non_empty),
        archived: meta.archived,
        created_at_ms,
        updated_at_ms: meta.updated_at,
        prompts,
        prompt_timestamps_ms,
        tokens_used: if token_usage_events.is_empty() {
            meta.tokens_used
                .and_then(|tokens| u64::try_from(tokens).ok())
        } else {
            Some(
                token_usage_events
                    .iter()
                    .fold(0u64, |total, event| total.saturating_add(event.tokens)),
            )
        },
        token_usage_events,
        message_count: meta.message_count.max(0) as u64,
        unknown_line_count: parsed.unknown_line_count,
    })
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

/// Wake adapters intentionally downgrade some filesystem/SQLite errors to an
/// empty list. Check each configured root first so common permission/read
/// failures become visible coverage errors instead of looking like zero use.
fn cached_summaries_for_roots(
    existing: &HashMap<String, ExistingSummary>,
    source: &str,
    roots: &[PathBuf],
    require_source_exists: bool,
) -> Vec<NativeSessionSummary> {
    existing
        .values()
        .filter_map(|stored| {
            if stored.summary.source != source {
                return None;
            }
            let source_path = PathBuf::from(wake_core::adapters::session_source_path(
                &stored.summary.source_path,
            ));
            let belongs_to_adapter = roots.iter().any(|root| {
                source_path.as_path() == root.as_path() || source_path.starts_with(root)
            });
            (belongs_to_adapter && (!require_source_exists || source_path.exists()))
                .then(|| stored.summary.clone())
        })
        .collect()
}

fn retain_cached_summaries(
    candidates: &mut BTreeMap<String, NativeSessionSummary>,
    coverage: &mut NativeUsageCoverage,
    cached: Vec<NativeSessionSummary>,
) {
    for summary in cached {
        coverage.unknown_lines = coverage
            .unknown_lines
            .saturating_add(summary.unknown_line_count as u64);
        coverage.partial |= summary.unknown_line_count > 0;
        insert_candidate(candidates, summary);
    }
}

fn check_adapter_roots(roots: &[PathBuf]) -> Result<()> {
    for root in roots {
        let metadata = match std::fs::metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect native source root {}", root.display()));
            }
        };
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&root)
                .with_context(|| format!("read native source root {}", root.display()))?
            {
                entry
                    .with_context(|| format!("enumerate native source root {}", root.display()))?;
            }
        } else {
            let mut source = std::fs::File::open(&root)
                .with_context(|| format!("open native source root {}", root.display()))?;
            let mut probe = [0u8; 1];
            source
                .read(&mut probe)
                .with_context(|| format!("read native source root {}", root.display()))?;
        }
    }
    Ok(())
}

fn source_fingerprint(file_ref: &SessionFileRef) -> String {
    let mut hasher = Sha256::new();
    // Bump this when the compact summary's interpretation changes. It forces
    // a one-time reparse after schema/metric changes such as archived state or
    // Wake's canonical session identity.
    hasher.update(b"zeron-native-usage-v4");
    hasher.update([0]);
    hasher.update(file_ref.agent.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(file_ref.file_path.as_bytes());
    hasher.update([0]);
    hasher.update(file_ref.mtime_ms.to_le_bytes());
    hasher.update(file_ref.size.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

fn insert_candidate(
    candidates: &mut BTreeMap<String, NativeSessionSummary>,
    candidate: NativeSessionSummary,
) {
    let replace = candidates.get(&candidate.key).is_none_or(|current| {
        (current.archived && !candidate.archived)
            || (current.archived == candidate.archived
                && candidate
                    .updated_at_ms
                    .cmp(&current.updated_at_ms)
                    .then_with(|| candidate.source_mtime_ms.cmp(&current.source_mtime_ms))
                    .then_with(|| candidate.source_size_bytes.cmp(&current.source_size_bytes))
                    .then_with(|| current.source_path.cmp(&candidate.source_path))
                    .is_gt())
    });
    if replace {
        candidates.insert(candidate.key.clone(), candidate);
    }
}

fn load_existing(connection: &Connection) -> Result<HashMap<String, ExistingSummary>> {
    let mut statement = connection.prepare(
        "SELECT key, agent, native_id, source, source_path, source_fingerprint,
                source_mtime_ms, source_size_bytes, project_path, project_name, model,
                archived, created_at_ms, updated_at_ms, prompts,
                prompt_timestamps_ms, tokens_used, message_count, unknown_line_count,
                token_usage_events
         FROM native_sessions",
    )?;
    let rows = statement.query_map([], |row| {
        let timestamps_json: String = row.get(15)?;
        let prompt_timestamps_ms = serde_json::from_str(&timestamps_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                15,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let source_size_bytes = row.get::<_, i64>(7)?.max(0) as u64;
        let tokens_used = row
            .get::<_, Option<i64>>(16)?
            .and_then(|tokens| u64::try_from(tokens).ok());
        let message_count = row.get::<_, i64>(17)?.max(0) as u64;
        let unknown_line_count = row.get::<_, i64>(18)?.max(0) as u32;
        let token_usage_events_json: String = row.get(19)?;
        let token_usage_events =
            serde_json::from_str(&token_usage_events_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    19,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        Ok(ExistingSummary {
            summary: NativeSessionSummary {
                key: row.get(0)?,
                agent: row.get(1)?,
                native_id: row.get(2)?,
                source: row.get(3)?,
                source_path: row.get(4)?,
                source_fingerprint: row.get(5)?,
                source_mtime_ms: row.get(6)?,
                source_size_bytes,
                project_path: row.get(8)?,
                project_name: row.get(9)?,
                model: row.get(10)?,
                archived: row.get::<_, i64>(11)? != 0,
                created_at_ms: row.get(12)?,
                updated_at_ms: row.get(13)?,
                prompts: row.get::<_, i64>(14)?.max(0) as u64,
                prompt_timestamps_ms,
                tokens_used,
                token_usage_events,
                message_count,
                unknown_line_count,
            },
        })
    })?;

    let mut existing = HashMap::new();
    for row in rows {
        let summary = row.context("read native usage index row")?;
        existing.insert(summary.summary.source_path.clone(), summary);
    }
    Ok(existing)
}

fn write_sessions<'a>(
    connection: &Connection,
    sessions: impl IntoIterator<Item = &'a NativeSessionSummary>,
) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM native_sessions", [])?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO native_sessions (
                 key, agent, native_id, source, source_path, source_fingerprint,
                 source_mtime_ms, source_size_bytes, project_path, project_name, model,
                 archived, created_at_ms, updated_at_ms, prompts,
                 prompt_timestamps_ms, tokens_used, message_count, unknown_line_count,
                 token_usage_events
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for summary in sessions {
            statement.execute(params![
                &summary.key,
                &summary.agent,
                &summary.native_id,
                &summary.source,
                &summary.source_path,
                &summary.source_fingerprint,
                summary.source_mtime_ms,
                i64::try_from(summary.source_size_bytes).unwrap_or(i64::MAX),
                &summary.project_path,
                &summary.project_name,
                &summary.model,
                summary.archived,
                summary.created_at_ms,
                summary.updated_at_ms,
                i64::try_from(summary.prompts).unwrap_or(i64::MAX),
                serde_json::to_string(&summary.prompt_timestamps_ms)?,
                summary
                    .tokens_used
                    .and_then(|tokens| i64::try_from(tokens).ok()),
                i64::try_from(summary.message_count).unwrap_or(i64::MAX),
                i64::from(summary.unknown_line_count),
                serde_json::to_string(&summary.token_usage_events)?,
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}
