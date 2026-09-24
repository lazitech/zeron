//! Read-only cumulative token usage observer for sessions owned by `pi-acp`.
//!
//! `pi-acp` does not currently forward pi's billing usage over ACP.  It does,
//! however, persist the exact native session path in
//! `~/.pi/pi-acp/session-map.json`.  This module follows that one path and
//! reads only the small metadata needed to reproduce pi's `getSessionStats()`
//! totals.  Transcript content is deliberately not deserialized or retained.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{self, Duration, MissedTickBehavior};
use zeron_proto::{AgentEvent, ContextUsage, SessionUsage};

use crate::HarnessError;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

type Event = Result<AgentEvent, HarnessError>;

/// A small controller for the background native-session reader.
pub(super) struct Observer {
    refresh_tx: mpsc::Sender<RefreshRequest>,
    worker: Option<JoinHandle<()>>,
}

struct RefreshRequest {
    done: oneshot::Sender<()>,
}

impl Observer {
    pub(super) fn start(
        session_id: String,
        session_map: Option<PathBuf>,
        context_directory: Option<PathBuf>,
        event_tx: mpsc::Sender<Event>,
    ) -> Self {
        let (refresh_tx, refresh_rx) = mpsc::channel(8);
        let worker = tokio::spawn(run_worker(
            session_id,
            session_map.unwrap_or_else(default_session_map_path),
            context_directory,
            event_tx,
            refresh_rx,
        ));
        Self {
            refresh_tx,
            worker: Some(worker),
        }
    }

    /// Request one immediate reader pass and wait until that pass completes.
    ///
    /// The worker owns all filesystem work.  A closed command channel, a
    /// dropped worker, or a closed event receiver simply makes this a no-op;
    /// shutdown must never wait on a dead observer.
    pub(super) async fn refresh(&self) {
        let _ = time::timeout(Duration::from_secs(2), async {
            let (done_tx, done_rx) = oneshot::channel();
            if self
                .refresh_tx
                .send(RefreshRequest { done: done_tx })
                .await
                .is_ok()
            {
                let _ = done_rx.await;
            }
        })
        .await;
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
    }
}

async fn run_worker(
    session_id: String,
    session_map: PathBuf,
    context_directory: Option<PathBuf>,
    event_tx: mpsc::Sender<Event>,
    mut refresh_rx: mpsc::Receiver<RefreshRequest>,
) {
    let mut reader = Reader::new(session_id, session_map);
    reader.context_directory = context_directory;
    let mut ticker = time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !reader.refresh(&event_tx).await {
                    break;
                }
            }
            request = refresh_rx.recv() => {
                let Some(request) = request else { break };
                let alive = reader.refresh(&event_tx).await;
                let _ = request.done.send(());
                if !alive {
                    break;
                }
            }
        }
    }
}

fn default_session_map_path() -> PathBuf {
    crate::executable::home_dir()
        .map(|home| home.join(".pi").join("pi-acp").join("session-map.json"))
        .unwrap_or_else(|| PathBuf::from(".pi/pi-acp/session-map.json"))
}

#[derive(Default)]
struct Reader {
    session_id: String,
    session_map: PathBuf,
    state: ReaderState,
    context_directory: Option<PathBuf>,
    last_context: Option<ContextUsage>,
}

impl Reader {
    fn new(session_id: String, session_map: PathBuf) -> Self {
        Self {
            session_id,
            session_map,
            state: ReaderState::default(),
            context_directory: None,
            last_context: None,
        }
    }

    async fn refresh(&mut self, event_tx: &mpsc::Sender<Event>) -> bool {
        let state = std::mem::take(&mut self.state);
        let session_id = self.session_id.clone();
        let session_map = self.session_map.clone();

        let directory = self.context_directory.clone();
        let result = tokio::task::spawn_blocking(move || {
            let context = directory
                .as_deref()
                .and_then(|directory| super::pi_context::read_snapshot(directory, &session_id));
            let (state, snapshot) = state.refresh_blocking(&session_id, &session_map);
            (state, snapshot, context)
        })
        .await;

        let Ok((state, snapshot, context)) = result else {
            // A runtime shutdown can cancel spawn_blocking.  Keep the worker
            // alive long enough to acknowledge an in-flight explicit refresh;
            // there is no useful filesystem result to emit.
            self.state = ReaderState::default();
            return true;
        };
        self.state = state;
        if let Some(usage) = context
            && self.last_context != Some(usage)
        {
            self.last_context = Some(usage);
            if event_tx
                .send(Ok(AgentEvent::ContextUsageSnapshot { usage }))
                .await
                .is_err()
            {
                return false;
            }
        }

        let Some(usage) = snapshot else {
            return true;
        };
        event_tx
            .send(Ok(AgentEvent::SessionUsage { usage }))
            .await
            .is_ok()
    }
}

#[derive(Default)]
struct ReaderState {
    file: Option<PathBuf>,
    identity: Option<FileIdentity>,
    offset: u64,
    pending: Vec<u8>,
    pending_start: u64,
    seen_ids: HashSet<EntryKey>,
    totals: Totals,
    last_emitted: Option<SessionUsage>,
    map_stamp: Option<FileStamp>,
    mapped_file: Option<PathBuf>,
    file_stamp: Option<FileStamp>,
    /// Small tail checkpoint distinguishes append from truncate-and-rewrite.
    anchor: Vec<u8>,
}

impl ReaderState {
    fn refresh_blocking(
        mut self,
        session_id: &str,
        session_map: &Path,
    ) -> (Self, Option<SessionUsage>) {
        let Ok(map_metadata) = fs::metadata(session_map) else {
            return (self, None);
        };
        let map_stamp = FileStamp::from_metadata(&map_metadata);
        if self.map_stamp.as_ref() != Some(&map_stamp) || self.mapped_file.is_none() {
            self.mapped_file = resolve_session_file(session_id, session_map);
            self.map_stamp = Some(map_stamp);
        }
        let Some(session_file) = self.mapped_file.clone() else {
            return (self, None);
        };

        // The native adapter stores an absolute path.  Relative paths are
        // accepted only as a convenience for hand-written/test maps and are
        // resolved relative to the map file's directory.
        let session_file = if session_file.is_absolute() {
            session_file
        } else {
            session_map
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(session_file)
        };

        // Resolve metadata again after relative-path normalization.
        let Ok(metadata) = fs::metadata(&session_file) else {
            return (self, None);
        };
        let identity = FileIdentity::from_metadata(&metadata);
        let stamp = FileStamp::from_metadata(&metadata);
        if self.file.as_ref() == Some(&session_file)
            && self.file_stamp.as_ref() == Some(&stamp)
            && anchor_matches(&session_file, self.offset, &self.anchor)
        {
            return (self, None);
        }

        let Ok((header_id, header_end)) = read_session_header(&session_file) else {
            self.clear();
            self.file = Some(session_file);
            self.identity = Some(identity);
            return (self, None);
        };
        if header_id != session_id {
            // Never let a stale map entry leak another session's totals.
            self.clear();
            self.file = Some(session_file);
            self.identity = Some(identity);
            return (self, None);
        }

        let file_changed = self.file.as_deref() != Some(session_file.as_path())
            || self.identity != Some(identity)
            || metadata.len() < self.offset
            || (self.file_stamp.is_some() && metadata.len() == self.offset)
            || !anchor_matches(&session_file, self.offset, &self.anchor);
        if file_changed {
            self.clear();
            self.file = Some(session_file.clone());
            self.identity = Some(identity);
            self.offset = header_end;
            self.pending_start = header_end;
        } else if self.offset < header_end {
            // This covers a state created before the header was fully flushed.
            self.offset = header_end;
            self.pending.clear();
            self.pending_start = header_end;
        }

        let Ok(mut file) = File::open(&session_file) else {
            return (self, None);
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return (self, None);
        }

        let read_start = self.offset;
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return (self, None);
        }
        self.offset = self.offset.saturating_add(bytes.len() as u64);

        if self.pending.is_empty() {
            self.pending_start = read_start;
        }
        self.pending.extend_from_slice(&bytes);
        self.consume_complete_lines();
        self.file_stamp = Some(stamp);
        let anchor_len = self.offset.min(128) as usize;
        self.anchor.resize(anchor_len, 0);
        if file
            .seek(SeekFrom::Start(self.offset - anchor_len as u64))
            .is_err()
            || file.read_exact(&mut self.anchor).is_err()
        {
            self.anchor.clear();
        }

        let snapshot = self.snapshot();
        let changed = snapshot.is_some() && snapshot != self.last_emitted;
        if changed {
            self.last_emitted = snapshot;
            (self, snapshot)
        } else {
            (self, None)
        }
    }

    fn clear(&mut self) {
        self.file = None;
        self.identity = None;
        self.offset = 0;
        self.pending.clear();
        self.pending_start = 0;
        self.seen_ids.clear();
        self.totals = Totals::default();
        self.last_emitted = None;
        self.file_stamp = None;
        self.anchor.clear();
    }

    fn consume_complete_lines(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        let mut consumed = 0usize;
        while let Some(rel_end) = pending[consumed..].iter().position(|b| *b == b'\n') {
            let end = consumed + rel_end;
            let line = &pending[consumed..end];
            let line_start = self.pending_start.saturating_add(consumed as u64);
            self.process_line(line, line_start);
            consumed = end + 1;
        }

        self.pending = pending[consumed..].to_vec();
        self.pending_start = self.pending_start.saturating_add(consumed as u64);
    }

    fn process_line(&mut self, line: &[u8], line_start: u64) {
        let line = trim_ascii_whitespace(line);
        if line.is_empty() {
            return;
        }
        let Ok(entry) = serde_json::from_slice::<SessionEntry>(line) else {
            // A complete malformed record may contain billable usage; never
            // present the remaining valid records as a complete total.
            self.totals.add_required_usage(None);
            return;
        };

        let key = entry
            .id
            .map(EntryKey::Id)
            .unwrap_or(EntryKey::Offset(line_start));
        if !self.seen_ids.insert(key) {
            return;
        }

        match entry.kind.as_str() {
            "usage" => self.totals.add_required_usage(entry.usage),
            "compaction" | "branch_summary" => {
                if entry.usage.is_some() {
                    self.totals.add_usage(entry.usage);
                }
            }
            "message" => {
                let Some(message) = entry.message else { return };
                match message.role.as_str() {
                    "assistant" => self.totals.add_required_usage(message.usage),
                    "toolResult" => {
                        if message.usage.is_some() {
                            self.totals.add_usage(message.usage);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> Option<SessionUsage> {
        if self.totals.usage_count == 0 {
            return None;
        }
        Some(SessionUsage {
            input_tokens: self.totals.input_known.then_some(self.totals.input),
            output_tokens: self.totals.output_known.then_some(self.totals.output),
            cached_input_tokens: self.totals.cache_known.then_some(self.totals.cache_read),
        })
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum EntryKey {
    Id(String),
    Offset(u64),
}

#[derive(Debug)]
struct Totals {
    input: u64,
    output: u64,
    cache_read: u64,
    input_known: bool,
    output_known: bool,
    cache_known: bool,
    usage_count: u64,
}

impl Default for Totals {
    fn default() -> Self {
        Self {
            input: 0,
            output: 0,
            cache_read: 0,
            input_known: true,
            output_known: true,
            cache_known: true,
            usage_count: 0,
        }
    }
}

impl Totals {
    fn add_required_usage(&mut self, usage: Option<Usage>) {
        let Some(usage) = usage else {
            self.usage_count = self.usage_count.saturating_add(1);
            self.input_known = false;
            self.output_known = false;
            self.cache_known = false;
            return;
        };
        self.add_usage(Some(usage));
    }

    fn add_usage(&mut self, usage: Option<Usage>) {
        let Some(usage) = usage else {
            self.input_known = false;
            self.output_known = false;
            self.cache_known = false;
            return;
        };
        self.usage_count = self.usage_count.saturating_add(1);

        if let Some(output) = usage.output {
            self.output = self.output.saturating_add(output);
        } else {
            self.output_known = false;
        }

        match (usage.input, usage.cache_read, usage.cache_write) {
            (Some(input), Some(cache_read), Some(cache_write)) => {
                self.input = self
                    .input
                    .saturating_add(input.saturating_add(cache_read).saturating_add(cache_write));
                self.cache_read = self.cache_read.saturating_add(cache_read);
            }
            _ => {
                self.input_known = false;
                self.cache_known = false;
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct SessionMap {
    version: Option<u64>,
    sessions: Option<HashMap<String, StoredSession>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSession {
    session_id: Option<String>,
    session_file: Option<PathBuf>,
}

fn resolve_session_file(session_id: &str, map_path: &Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(map_path).ok()?;
    let map = serde_json::from_str::<SessionMap>(&raw).ok()?;
    if map.version != Some(1) {
        return None;
    }
    let sessions = map.sessions?;
    let stored = sessions.get(session_id)?;
    if stored.session_id.as_deref() != Some(session_id) {
        return None;
    }
    stored.session_file.clone()
}

#[derive(Debug, Deserialize)]
struct SessionHeader {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<String>,
}

fn read_session_header(path: &Path) -> std::io::Result<(String, u64)> {
    let file = File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = reader.read_until(b'\n', &mut line)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing pi session header",
            ));
        }
        if trim_ascii_whitespace(&line).is_empty() {
            continue;
        }
        let header = serde_json::from_slice::<SessionHeader>(trim_ascii_whitespace(&line))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid header"))?;
        if header.kind.as_deref() != Some("session") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "wrong pi session header type",
            ));
        }
        let id = header.id.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "missing pi session id")
        })?;
        return Ok((id, reader.stream_position()?));
    }
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while let Some(first) = bytes.first() {
        if !first.is_ascii_whitespace() {
            break;
        }
        bytes = &bytes[1..];
    }
    while let Some(last) = bytes.last() {
        if !last.is_ascii_whitespace() {
            break;
        }
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    identity: FileIdentity,
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            identity: FileIdentity::from_metadata(metadata),
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

fn anchor_matches(path: &Path, offset: u64, anchor: &[u8]) -> bool {
    if anchor.is_empty() {
        return true;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut bytes = vec![0; anchor.len()];
    file.seek(SeekFrom::Start(offset.saturating_sub(anchor.len() as u64)))
        .is_ok()
        && file.read_exact(&mut bytes).is_ok()
        && bytes == anchor
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(not(unix))]
    Portable {
        len: u64,
        modified: Option<SystemTime>,
    },
}

impl FileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self::Unix {
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self::Portable {
                len: metadata.len(),
                modified: metadata.modified().ok(),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionEntry {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    usage: Option<Usage>,
    message: Option<SessionMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionMessage {
    role: String,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Usage {
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    fn map_file(dir: &Path, session_id: &str, session_file: &Path) -> PathBuf {
        let map = dir.join("session-map.json");
        let value = serde_json::json!({
            "version": 1,
            "sessions": {
                session_id: {
                    "sessionId": session_id,
                    "cwd": dir,
                    "sessionFile": session_file,
                    "updatedAt": "2026-09-23T00:00:00.000Z"
                }
            }
        });
        fs::write(&map, serde_json::to_vec(&value).unwrap()).unwrap();
        map
    }

    fn header(session_id: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "session",
                "version": 3,
                "id": session_id,
                "cwd": "/tmp/pi"
            })
        )
    }

    fn assistant(id: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) -> String {
        serde_json::json!({
            "type": "message",
            "id": id,
            "parentId": null,
            "timestamp": "2026-09-23T00:00:00.000Z",
            "message": {
                "role": "assistant",
                "content": [],
                "usage": {
                    "input": input,
                    "output": output,
                    "cacheRead": cache_read,
                    "cacheWrite": cache_write,
                    "totalTokens": input + output + cache_read + cache_write,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}
                }
            }
        })
        .to_string()
            + "\n"
    }

    fn usage_entry(id: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) -> String {
        serde_json::json!({
            "type": "usage",
            "id": id,
            "usage": {
                "input": input,
                "output": output,
                "cacheRead": cache_read,
                "cacheWrite": cache_write
            }
        })
        .to_string()
            + "\n"
    }

    fn tool_result(id: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) -> String {
        serde_json::json!({
            "type": "message",
            "id": id,
            "message": {
                "role": "toolResult",
                "content": [],
                "usage": {
                    "input": input,
                    "output": output,
                    "cacheRead": cache_read,
                    "cacheWrite": cache_write
                }
            }
        })
        .to_string()
            + "\n"
    }

    fn run_reader(mut reader: Reader) -> (Reader, Option<SessionUsage>) {
        let session_id = reader.session_id.clone();
        let session_map = reader.session_map.clone();
        let (state, snapshot) = reader.state.refresh_blocking(&session_id, &session_map);
        reader.state = state;
        (reader, snapshot)
    }

    #[test]
    fn initial_and_resume_totals_include_cache_in_input() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        fs::write(&session, header(id) + &assistant("a", 100, 20, 80, 0)).unwrap();
        let map = map_file(dir.path(), id, &session);
        let reader = Reader::new(id.into(), map);
        let (reader, usage) = run_reader(reader);
        assert_eq!(
            usage,
            Some(SessionUsage {
                input_tokens: Some(180),
                output_tokens: Some(20),
                cached_input_tokens: Some(80),
            })
        );

        let (_, usage) = run_reader(reader);
        assert_eq!(usage, None);
    }

    #[test]
    fn append_duplicate_and_partial_lines_are_incremental() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        fs::write(&session, header(id) + &assistant("a", 100, 20, 80, 0)).unwrap();
        let map = map_file(dir.path(), id, &session);
        let reader = Reader::new(id.into(), map);
        let (mut reader, first) = run_reader(reader);
        assert!(first.is_some());

        let second = assistant("a", 100, 20, 80, 0);
        fs::OpenOptions::new()
            .append(true)
            .open(&session)
            .unwrap()
            .write_all(second[..second.len() - 1].as_bytes())
            .unwrap();
        let (reader2, usage) = run_reader(reader);
        reader = reader2;
        assert_eq!(usage, None);

        fs::OpenOptions::new()
            .append(true)
            .open(&session)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let (reader, usage) = run_reader(reader);
        assert_eq!(usage, None, "duplicate id must not add totals");

        fs::OpenOptions::new()
            .append(true)
            .open(&session)
            .unwrap()
            .write_all(assistant("b", 50, 5, 40, 0).as_bytes())
            .unwrap();
        let (_, usage) = run_reader(reader);
        assert_eq!(
            usage,
            Some(SessionUsage {
                input_tokens: Some(270),
                output_tokens: Some(25),
                cached_input_tokens: Some(120),
            })
        );
    }

    #[test]
    fn compaction_and_branch_summary_usage_are_counted() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        let compaction = serde_json::json!({
            "type": "compaction", "id": "c", "parentId": null,
            "usage": {"input": 10, "output": 2, "cacheRead": 3, "cacheWrite": 1}
        });
        let branch = serde_json::json!({
            "type": "branch_summary", "id": "b", "parentId": "c",
            "usage": {"input": 20, "output": 4, "cacheRead": 5, "cacheWrite": 0}
        });
        fs::write(
            &session,
            header(id) + &compaction.to_string() + "\n" + &branch.to_string() + "\n",
        )
        .unwrap();
        let map = map_file(dir.path(), id, &session);
        let (_, usage) = run_reader(Reader::new(id.into(), map));
        assert_eq!(
            usage,
            Some(SessionUsage {
                input_tokens: Some(39),
                output_tokens: Some(6),
                cached_input_tokens: Some(8),
            })
        );
    }

    #[test]
    fn all_billable_entry_kinds_are_counted_once() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        fs::write(
            &session,
            header(id)
                + &assistant("assistant", 10, 1, 2, 0)
                + &usage_entry("usage", 20, 3, 4, 1)
                + &tool_result("tool", 7, 2, 1, 0),
        )
        .unwrap();
        let map = map_file(dir.path(), id, &session);
        let (_, usage) = run_reader(Reader::new(id.into(), map));
        assert_eq!(
            usage,
            Some(SessionUsage {
                input_tokens: Some(45),
                output_tokens: Some(6),
                cached_input_tokens: Some(7),
            })
        );
    }

    #[test]
    fn missing_required_usage_is_unknown_but_optional_tool_usage_is_ignored() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        let missing_assistant_usage = serde_json::json!({
            "type": "message",
            "id": "assistant-missing",
            "message": {"role": "assistant", "content": []}
        });
        let missing_tool_usage = serde_json::json!({
            "type": "message",
            "id": "tool-missing",
            "message": {"role": "toolResult", "content": []}
        });
        fs::write(
            &session,
            header(id)
                + &missing_assistant_usage.to_string()
                + "\n"
                + &missing_tool_usage.to_string()
                + "\n"
                + &assistant("known", 10, 2, 3, 0),
        )
        .unwrap();
        let map = map_file(dir.path(), id, &session);
        let (_, usage) = run_reader(Reader::new(id.into(), map));
        assert_eq!(
            usage,
            Some(SessionUsage {
                input_tokens: None,
                output_tokens: None,
                cached_input_tokens: None,
            })
        );
    }

    #[test]
    fn missing_cache_is_unknown_and_does_not_become_zero() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        let missing_cache = serde_json::json!({
            "type": "message", "id": "a", "parentId": null,
            "message": {"role": "assistant", "content": [], "usage": {"input": 100, "output": 20}}
        });
        fs::write(&session, header(id) + &missing_cache.to_string() + "\n").unwrap();
        let map = map_file(dir.path(), id, &session);
        let (_, usage) = run_reader(Reader::new(id.into(), map));
        assert_eq!(
            usage,
            Some(SessionUsage {
                input_tokens: None,
                output_tokens: Some(20),
                cached_input_tokens: None,
            })
        );
    }

    #[test]
    fn truncate_replace_and_header_mismatch_reset_reader() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        fs::write(&session, header(id) + &assistant("a", 100, 20, 80, 0)).unwrap();
        let map = map_file(dir.path(), id, &session);
        let reader = Reader::new(id.into(), map);
        let (reader, first) = run_reader(reader);
        assert!(first.is_some());

        fs::write(&session, header(id) + &assistant("b", 10, 2, 5, 0)).unwrap();
        let (reader, replacement) = run_reader(reader);
        assert_eq!(
            replacement,
            Some(SessionUsage {
                input_tokens: Some(15),
                output_tokens: Some(2),
                cached_input_tokens: Some(5),
            })
        );

        fs::write(
            &session,
            header("other") + &assistant("c", 999, 999, 999, 999),
        )
        .unwrap();
        let (_, mismatch) = run_reader(reader);
        assert_eq!(mismatch, None);
    }

    #[test]
    fn mapping_is_exact_and_does_not_scan_other_sessions() {
        let dir = tempdir().unwrap();
        let id = "pi-session";
        let session = dir.path().join("session.jsonl");
        let other = dir.path().join("other.jsonl");
        fs::write(&session, header(id) + &assistant("a", 100, 20, 80, 0)).unwrap();
        fs::write(&other, header("other") + &assistant("b", 900, 90, 700, 0)).unwrap();
        let map = map_file(dir.path(), id, &session);
        let (_, usage) = run_reader(Reader::new(id.into(), map));
        assert_eq!(
            usage,
            Some(SessionUsage {
                input_tokens: Some(180),
                output_tokens: Some(20),
                cached_input_tokens: Some(80),
            })
        );
    }
}
