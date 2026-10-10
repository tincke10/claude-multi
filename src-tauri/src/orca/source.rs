//! Edge I/O for reading Orca's state (macOS). Strictly read-only: the Orca
//! CLI is only ever asked to list things, and Orca's own files are never
//! opened for writing (the SQLite fallback works on a private copy).
//!
//! Order: CLI first (live, public interface), SQLite snapshot as fallback.

use crate::orca::derive::canon;
use crate::orca::model::{
    parse_profile_index, parse_repo_list, parse_repos_payload, parse_settings_payload,
    parse_status, parse_worktree_list, parse_worktree_meta_payload, OrcaRepo, OrcaSettings,
    OrcaSnapshot, OrcaWorktree, Source,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Where the Orca app ships its CLI (a bash script, fine on a GUI `PATH`).
pub const CLI_PATH: &str = "/Applications/Orca.app/Contents/Resources/bin/orca";

/// Worktree list sizes to request, smallest first. The CLI has `--limit` but
/// no cursor, so a truncated answer is retried bigger, up to the last entry.
const WORKTREE_LIMITS: [u32; 2] = [500, 2000];

/// Cap on captured output per stream, so a runaway CLI cannot eat memory.
const MAX_OUTPUT: u64 = 32 * 1024 * 1024;

/// Locations of Orca's CLI and data directory (injectable for tests).
#[derive(Debug, Clone)]
pub struct OrcaPaths {
    pub cli: PathBuf,
    /// `~/Library/Application Support/orca`
    pub data_dir: PathBuf,
}

impl OrcaPaths {
    pub fn system(home: &Path) -> Self {
        OrcaPaths {
            cli: PathBuf::from(CLI_PATH),
            data_dir: home.join("Library/Application Support/orca"),
        }
    }
}

/// Per-call CLI timeouts.
#[derive(Debug, Clone)]
pub struct Timeouts {
    pub status: Duration,
    pub list: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            status: Duration::from_secs(3),
            list: Duration::from_secs(8),
        }
    }
}

fn spawn_reader<R: Read + Send + 'static>(stream: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(s) = stream {
            let _ = s.take(MAX_OUTPUT).read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// Runs the Orca CLI by absolute path with an argv vector (no shell), stdin
/// closed, and a hard timeout. Both streams are drained on threads while the
/// child runs, so output bigger than the pipe buffer cannot deadlock it.
pub fn run_cli(cli: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", cli.display()))?;
    let out_rx = spawn_reader(child.stdout.take());
    let err_rx = spawn_reader(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                // Not joining the readers: a grandchild may still hold the pipe.
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "`orca {}` timed out after {}s",
                    args.join(" "),
                    timeout.as_secs_f32()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("waiting for orca failed: {e}")),
        }
    };
    let grace = Duration::from_secs(2);
    let stdout = out_rx.recv_timeout(grace).unwrap_or_default();
    let stderr = err_rx.recv_timeout(grace).unwrap_or_default();
    if !status.success() {
        let tail = String::from_utf8_lossy(&stderr);
        let tail = tail.trim();
        let tail: String = tail
            .chars()
            .rev()
            .take(300)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!(
            "`orca {}` failed ({status}): {tail}",
            args.join(" ")
        ));
    }
    String::from_utf8(stdout).map_err(|_| "orca printed non-UTF-8 output".to_string())
}

fn read_via_cli(paths: &OrcaPaths, t: &Timeouts) -> Result<OrcaSnapshot, String> {
    let cli = paths.cli.as_path();
    if !parse_status(&run_cli(cli, &["status", "--json"], t.status)?)? {
        return Err("Orca is not running".to_string());
    }
    let repos = parse_repo_list(&run_cli(cli, &["repo", "list", "--json"], t.list)?)?;

    let mut worktrees = Vec::new();
    let mut truncated = false;
    for limit in WORKTREE_LIMITS {
        let limit = limit.to_string();
        let raw = run_cli(
            cli,
            &["worktree", "list", "--json", "--limit", &limit],
            t.list,
        )?;
        (worktrees, truncated) = parse_worktree_list(&raw)?;
        if !truncated {
            break;
        }
    }
    Ok(OrcaSnapshot {
        source: Source::Cli,
        repos,
        worktrees,
        truncated,
        settings: None,
        other_profiles: 0,
        runtime_ready: true,
    })
}

// ---- SQLite fallback ------------------------------------------------------
//
// Orca keeps its state in `profiles/<id>/profile-state.db` in WAL mode while
// running. Opening that file ourselves, even read-only, would take WAL
// read-locks and may create or recover the `-shm` file in Orca's directory,
// and `immutable=1` would ignore the WAL (stale data). So: copy the db and
// its `-wal` (never `-shm`) into a private temp dir using plain read-only file
// reads, and open the COPY. Orca's own files are only ever `File::open`ed.

/// The only schema version of any document domain we understand.
const SUPPORTED_DOMAIN_VERSION: i64 = 1;

/// A private copy of the active profile's database, opened.
struct ProfileSnapshot {
    // Field order matters: the connection must drop before its directory.
    conn: Connection,
    _dir: tempfile::TempDir,
    other_profiles: usize,
}

type FileStamp = Option<(u64, std::time::SystemTime)>;

fn stamp(path: &Path) -> FileStamp {
    let md = std::fs::metadata(path).ok()?;
    Some((md.len(), md.modified().ok()?))
}

fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    // Not `fs::copy`: it would carry over a read-only mode, and the copy must
    // be writable for SQLite to replay the WAL into it.
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::File::create(to)?;
    std::io::copy(&mut src, &mut dst)?;
    Ok(())
}

/// Copies db + wal into a fresh temp dir, retrying (up to 3 times) when either
/// file changed while copying. After the last attempt the copy is used as is:
/// WAL frames are checksummed, so a torn tail is ignored on replay.
fn copy_snapshot(db: &Path) -> Result<tempfile::TempDir, String> {
    let mut wal_name = db.file_name().unwrap_or_default().to_os_string();
    wal_name.push("-wal");
    let wal = db.with_file_name(wal_name);
    let mut last = None;
    for _ in 0..3 {
        let before = (stamp(db), stamp(&wal));
        let dir = tempfile::tempdir().map_err(|e| format!("no temp dir: {e}"))?;
        copy_file(db, &dir.path().join("profile-state.db"))
            .map_err(|e| format!("could not read {}: {e}", db.display()))?;
        if before.1.is_some() {
            // The wal may vanish between stamp and copy (Orca checkpointed).
            let _ = copy_file(&wal, &dir.path().join("profile-state.db-wal"));
        }
        let stable = before == (stamp(db), stamp(&wal));
        last = Some(dir);
        if stable {
            break;
        }
    }
    last.ok_or_else(|| "snapshot failed".to_string())
}

fn open_profile_snapshot(paths: &OrcaPaths) -> Result<ProfileSnapshot, String> {
    let index = paths.data_dir.join("orca-profile-index.json");
    let (profile, count) = match std::fs::read_to_string(&index) {
        Ok(raw) => parse_profile_index(&raw)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ("local-default".to_string(), 0),
        Err(e) => return Err(format!("could not read {}: {e}", index.display())),
    };
    let db = paths
        .data_dir
        .join("profiles")
        .join(&profile)
        .join("profile-state.db");
    if !db.is_file() {
        return Err(format!("no Orca database at {}", db.display()));
    }
    let dir = copy_snapshot(&db)?;
    let conn = Connection::open_with_flags(
        dir.path().join("profile-state.db"),
        // Read-write on OUR copy so the WAL can be replayed into it.
        OpenFlags::SQLITE_OPEN_READ_WRITE,
    )
    .map_err(|e| format!("could not open the Orca database copy: {e}"))?;
    let _ = conn.busy_timeout(Duration::from_secs(2));
    Ok(ProfileSnapshot {
        conn,
        _dir: dir,
        other_profiles: count.saturating_sub(1),
    })
}

impl ProfileSnapshot {
    /// `(domain_version, payload)` of one document, if present.
    fn document(&self, domain: &str) -> Result<Option<(i64, String)>, String> {
        self.conn
            .query_row(
                "SELECT domain_version, payload FROM profile_state_documents WHERE domain = ?1",
                [domain],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| format!("could not read the Orca database: {e}"))
    }

    /// A required document at the supported schema version.
    fn required(&self, domain: &str) -> Result<String, String> {
        match self.document(domain)? {
            Some((SUPPORTED_DOMAIN_VERSION, payload)) => Ok(payload),
            Some((v, _)) => Err(format!("unsupported schema ({domain} v{v})")),
            None => Err(format!("`{domain}` missing from the Orca database")),
        }
    }

    /// Settings are optional: any problem just means "unknown".
    fn settings(&self) -> Option<OrcaSettings> {
        match self.document("settings").ok()?? {
            (SUPPORTED_DOMAIN_VERSION, payload) => parse_settings_payload(&payload).ok(),
            _ => None,
        }
    }

    fn repos_and_worktrees(&self) -> Result<(Vec<OrcaRepo>, Vec<OrcaWorktree>), String> {
        let repos = parse_repos_payload(&self.required("repos")?)?;
        let meta = parse_worktree_meta_payload(&self.required("worktreeMeta")?)?;
        let worktrees = meta
            .into_iter()
            .filter_map(|mut w| {
                let repo = repos.iter().find(|r| r.id == w.repo_id)?;
                let path = Path::new(&w.path);
                // The meta keeps entries for worktrees long deleted.
                if !path.is_dir() {
                    return None;
                }
                w.is_main = canon(path) == canon(Path::new(&repo.path));
                Some(w)
            })
            .collect();
        Ok((repos, worktrees))
    }
}

fn read_via_sqlite(paths: &OrcaPaths) -> Result<OrcaSnapshot, String> {
    let snap = open_profile_snapshot(paths)?;
    let (repos, worktrees) = snap.repos_and_worktrees()?;
    Ok(OrcaSnapshot {
        source: Source::Sqlite,
        repos,
        worktrees,
        truncated: false,
        settings: snap.settings(),
        other_profiles: snap.other_profiles,
        runtime_ready: false,
    })
}

/// Reads Orca's state with default timeouts.
pub fn read_snapshot(paths: &OrcaPaths) -> Result<OrcaSnapshot, String> {
    read_snapshot_with(paths, &Timeouts::default())
}

/// CLI first (live data), the SQLite snapshot as fallback. Settings always
/// come from SQLite since the CLI exposes none. `Err("Orca state
/// unavailable: ...")` when neither source can answer.
pub fn read_snapshot_with(paths: &OrcaPaths, t: &Timeouts) -> Result<OrcaSnapshot, String> {
    match read_via_cli(paths, t) {
        Ok(mut snap) => {
            if let Ok(db) = open_profile_snapshot(paths) {
                snap.settings = db.settings();
                snap.other_profiles = db.other_profiles;
            }
            Ok(snap)
        }
        Err(cli_err) => read_via_sqlite(paths)
            .map_err(|db_err| format!("Orca state unavailable: {cli_err}; database: {db_err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orca::fixtures::*;
    use crate::orca::model::Source;
    use std::path::PathBuf;
    use std::time::Duration;

    fn paths_for(cli: &FakeCli) -> OrcaPaths {
        OrcaPaths {
            cli: cli.path.clone(),
            // No database anywhere: only the CLI can answer.
            data_dir: cli.dir.path().join("no-orca-data"),
        }
    }

    fn fast() -> Timeouts {
        Timeouts {
            status: Duration::from_secs(5),
            list: Duration::from_secs(5),
        }
    }

    #[test]
    fn test_should_return_cli_snapshot_when_cli_answers() {
        let cli = FakeCli::healthy();
        let snap = read_snapshot_with(&paths_for(&cli), &fast()).unwrap();
        assert_eq!(snap.source, Source::Cli);
        assert!(snap.runtime_ready);
        assert_eq!(snap.repos.len(), 2);
        assert_eq!(snap.worktrees.len(), 3);
        assert!(!snap.truncated);
        // No profile database: settings are simply unknown.
        assert!(snap.settings.is_none());
        let calls = cli.calls();
        assert_eq!(calls[0], "status --json");
        assert_eq!(calls[1], "repo list --json");
        assert_eq!(calls[2], "worktree list --json --limit 500");
    }

    #[test]
    fn test_should_request_larger_limit_when_cli_reports_truncated() {
        let cli = FakeCli::healthy();
        cli.put("worktrees_small.json", WORKTREES_TRUNCATED_JSON);
        let snap = read_snapshot_with(&paths_for(&cli), &fast()).unwrap();
        assert_eq!(snap.worktrees.len(), 3, "full list from the larger request");
        assert!(!snap.truncated);
        let wt_calls: Vec<_> = cli
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("worktree"))
            .collect();
        assert_eq!(
            wt_calls,
            vec![
                "worktree list --json --limit 500",
                "worktree list --json --limit 2000"
            ]
        );
    }

    #[test]
    fn test_should_flag_partial_when_still_truncated_at_the_cap() {
        let cli = FakeCli::healthy();
        cli.put("worktrees.json", WORKTREES_TRUNCATED_JSON);
        cli.put("worktrees_small.json", WORKTREES_TRUNCATED_JSON);
        let snap = read_snapshot_with(&paths_for(&cli), &fast()).unwrap();
        assert!(snap.truncated);
        assert_eq!(snap.worktrees.len(), 1);
        assert_eq!(
            cli.calls()
                .iter()
                .filter(|c| c.starts_with("worktree"))
                .count(),
            2,
            "stops at the cap"
        );
    }

    #[test]
    fn test_should_be_unavailable_when_cli_missing_exits_nonzero_or_prints_garbage() {
        let missing = OrcaPaths {
            cli: PathBuf::from("/nonexistent/orca"),
            data_dir: PathBuf::from("/nonexistent/data"),
        };
        assert!(read_snapshot_with(&missing, &fast()).is_err());

        let failing = FakeCli::with_script("echo boom >&2; exit 1");
        assert!(read_snapshot_with(&paths_for(&failing), &fast()).is_err());

        let garbage = FakeCli::with_script("echo 'not json'");
        assert!(read_snapshot_with(&paths_for(&garbage), &fast()).is_err());
    }

    #[test]
    fn test_should_be_unavailable_when_cli_runtime_not_ready() {
        let cli = FakeCli::healthy();
        cli.put("status.json", STATUS_STARTING);
        let err = read_snapshot_with(&paths_for(&cli), &fast()).unwrap_err();
        assert!(err.to_lowercase().contains("unavailable"), "{err}");
        assert!(
            !cli.calls().iter().any(|c| c.starts_with("repo")),
            "no listing while Orca is not ready"
        );
    }

    #[test]
    fn test_should_not_deadlock_when_stdout_exceeds_pipe_buffer() {
        let cli = FakeCli::healthy();
        let repos: Vec<String> = (0..2500)
            .map(|i| {
                format!(r#"{{"id":"r{i}","path":"/Users/test/code/repo-{i}","displayName":"repo-{i}","pad":"{}"}}"#, "x".repeat(40))
            })
            .collect();
        let big = format!(
            r#"{{"ok":true,"result":{{"repos":[{}]}}}}"#,
            repos.join(",")
        );
        assert!(big.len() > 200_000);
        cli.put("repos.json", &big);
        let snap = read_snapshot_with(&paths_for(&cli), &fast()).unwrap();
        assert_eq!(snap.repos.len(), 2500);
    }

    #[test]
    fn test_should_time_out_when_cli_hangs() {
        let cli = FakeCli::with_script("exec sleep 5");
        let started = std::time::Instant::now();
        let t = Timeouts {
            status: Duration::from_millis(300),
            list: Duration::from_millis(300),
        };
        assert!(read_snapshot_with(&paths_for(&cli), &t).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn test_should_report_stderr_when_cli_exits_nonzero() {
        let cli = FakeCli::with_script("echo 'kaboom' >&2; exit 4");
        let err = run_cli(&cli.path, &["status", "--json"], Duration::from_secs(5)).unwrap_err();
        assert!(err.contains("kaboom"), "{err}");
    }

    // ---- SQLite fallback -------------------------------------------------

    use std::path::Path;

    fn missing_cli(data_dir: &Path) -> OrcaPaths {
        OrcaPaths {
            cli: PathBuf::from("/nonexistent/orca"),
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// Synthetic world under `base`: repo-a (main dir + one live worktree + one
    /// stale meta entry) and repo-b. Returns (repos, meta) payloads.
    fn world(base: &Path) -> (String, String) {
        let a = base.join("code/repo-a");
        let b = base.join("code/repo-b");
        let live = base.join("wts/repo-a/live");
        for d in [&a, &b, &live] {
            std::fs::create_dir_all(d).unwrap();
        }
        let stale = base.join("wts/repo-a/stale-gone");
        let repos = format!(
            r#"[{{"id":"r1","path":"{}","displayName":"repo-a"}},{{"id":"r2","path":"{}","displayName":"repo-b"}}]"#,
            a.display(),
            b.display()
        );
        let meta = format!(
            r#"{{"r1::{}":{{}},"r1::{}":{{}},"r1::{}":{{}},"r2::{}":{{}}}}"#,
            a.display(),
            live.display(),
            stale.display(),
            b.display()
        );
        (repos, meta)
    }

    fn settings_json(base: &Path) -> String {
        format!(
            r#"{{"workspaceDir":"{}/wts","nestWorkspaces":true,"agentCmdOverrides":{{"claude":"cms --auto"}}}}"#,
            base.display()
        )
    }

    #[test]
    fn test_should_read_from_sqlite_when_cli_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        write_profile_index(&data, "p1", &["p1"]);
        write_profile_db(
            &data,
            "p1",
            &standard_docs(&repos, &meta, &settings_json(tmp.path())),
            &[],
        );

        let snap = read_snapshot_with(&missing_cli(&data), &fast()).unwrap();
        assert_eq!(snap.source, Source::Sqlite);
        assert!(!snap.runtime_ready);
        assert_eq!(snap.repos.len(), 2);
        // Stale meta entry (directory gone) is dropped; the main worktree is
        // recognised by its path equalling the repo path.
        assert_eq!(snap.worktrees.len(), 3);
        let main: Vec<_> = snap.worktrees.iter().filter(|w| w.is_main).collect();
        assert_eq!(main.len(), 2);
        assert!(snap
            .worktrees
            .iter()
            .all(|w| !w.path.contains("stale-gone")));
        let s = snap.settings.unwrap();
        assert_eq!(s.claude_cmd_override.as_deref(), Some("cms --auto"));
        assert!(s.nest_workspaces);
    }

    #[test]
    fn test_should_fall_back_to_sqlite_when_cli_exits_nonzero_or_prints_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        write_profile_index(&data, "p1", &["p1"]);
        write_profile_db(&data, "p1", &standard_docs(&repos, &meta, "{}"), &[]);
        for script in ["exit 1", "echo 'not json'"] {
            let cli = FakeCli::with_script(script);
            let paths = OrcaPaths {
                cli: cli.path.clone(),
                data_dir: data.clone(),
            };
            let snap = read_snapshot_with(&paths, &fast()).unwrap();
            assert_eq!(snap.source, Source::Sqlite, "{script}");
        }
    }

    #[test]
    fn test_should_take_settings_from_sqlite_when_cli_answers() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        write_profile_index(&data, "p1", &["p1", "p2"]);
        write_profile_db(
            &data,
            "p1",
            &standard_docs(&repos, &meta, &settings_json(tmp.path())),
            &[],
        );
        let cli = FakeCli::healthy();
        let paths = OrcaPaths {
            cli: cli.path.clone(),
            data_dir: data,
        };
        let snap = read_snapshot_with(&paths, &fast()).unwrap();
        assert_eq!(snap.source, Source::Cli);
        assert_eq!(snap.repos.len(), 2, "repos still come from the CLI");
        assert!(snap.settings.unwrap().nest_workspaces);
        assert_eq!(snap.other_profiles, 1);
    }

    #[test]
    fn test_should_read_only_the_active_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        let other = r#"[{"id":"x","path":"/Users/test/other","displayName":"other"}]"#;
        write_profile_index(&data, "p2", &["p1", "p2", "p3"]);
        write_profile_db(&data, "p1", &standard_docs(other, "{}", "{}"), &[]);
        write_profile_db(&data, "p2", &standard_docs(&repos, &meta, "{}"), &[]);
        write_profile_db(&data, "p3", &standard_docs(other, "{}", "{}"), &[]);
        let snap = read_snapshot_with(&missing_cli(&data), &fast()).unwrap();
        assert_eq!(snap.repos.len(), 2);
        assert!(snap.repos.iter().all(|r| r.display_name != "other"));
        assert_eq!(snap.other_profiles, 2);
    }

    #[test]
    fn test_should_default_to_local_default_profile_when_index_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        write_profile_db(
            &data,
            "local-default",
            &standard_docs(&repos, &meta, "{}"),
            &[],
        );
        let snap = read_snapshot_with(&missing_cli(&data), &fast()).unwrap();
        assert_eq!(snap.repos.len(), 2);
    }

    #[test]
    fn test_should_be_unavailable_when_schema_version_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        write_profile_index(&data, "p1", &["p1"]);
        let docs = vec![
            ("repos", repos.as_str(), 2),
            ("worktreeMeta", meta.as_str(), 1),
        ];
        write_profile_db(&data, "p1", &docs, &[]);
        let err = read_snapshot_with(&missing_cli(&data), &fast()).unwrap_err();
        assert!(err.contains("unsupported schema"), "{err}");
    }

    #[test]
    fn test_should_drop_only_settings_when_settings_schema_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        write_profile_index(&data, "p1", &["p1"]);
        let docs = vec![
            ("repos", repos.as_str(), 1),
            ("worktreeMeta", meta.as_str(), 1),
            ("settings", "{}", 2),
        ];
        write_profile_db(&data, "p1", &docs, &[]);
        let snap = read_snapshot_with(&missing_cli(&data), &fast()).unwrap();
        assert_eq!(snap.repos.len(), 2);
        assert!(snap.settings.is_none());
    }

    #[test]
    fn test_should_be_unavailable_when_db_corrupt_absent_or_profile_id_unsafe() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");

        // Nothing at all.
        assert!(read_snapshot_with(&missing_cli(&data), &fast()).is_err());

        // Corrupt database file.
        write_profile_index(&data, "p1", &["p1"]);
        let dir = data.join("profiles/p1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("profile-state.db"), vec![7u8; 4096]).unwrap();
        let err = read_snapshot_with(&missing_cli(&data), &fast()).unwrap_err();
        assert!(err.to_lowercase().contains("unavailable"), "{err}");

        // Path-traversal profile id is refused before touching the disk.
        write_profile_index(&data, "../escape", &["p1"]);
        assert!(read_snapshot_with(&missing_cli(&data), &fast()).is_err());
    }

    #[test]
    fn test_should_see_uncheckpointed_wal_rows_when_snapshotting() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        let in_wal = r#"[{"id":"r9","path":"/Users/test/only-in-wal","displayName":"wal-repo"}]"#;
        write_profile_index(&data, "p1", &["p1"]);
        let dir = write_profile_db(
            &data,
            "p1",
            &standard_docs(&repos, &meta, "{}"),
            &[("repos", in_wal)],
        );
        assert!(
            std::fs::metadata(dir.join("profile-state.db-wal"))
                .unwrap()
                .len()
                > 0,
            "fixture must leave data only in the WAL"
        );
        let snap = read_snapshot_with(&missing_cli(&data), &fast()).unwrap();
        assert_eq!(snap.repos.len(), 1);
        assert_eq!(snap.repos[0].display_name, "wal-repo");
    }

    fn fingerprint(dir: &Path) -> Vec<(String, u64, std::time::SystemTime, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let md = e.metadata().unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    md.len(),
                    md.modified().unwrap(),
                    if md.is_file() {
                        std::fs::read(e.path()).unwrap()
                    } else {
                        Vec::new()
                    },
                )
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    #[test]
    fn test_should_not_modify_original_db_or_wal_when_snapshot_read() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("orca");
        let (repos, meta) = world(tmp.path());
        write_profile_index(&data, "p1", &["p1"]);
        let dir = write_profile_db(
            &data,
            "p1",
            &standard_docs(&repos, &meta, "{}"),
            &[("settings", r#"{"nestWorkspaces":true}"#)],
        );
        // Read-only files prove the read never needs write access to them.
        for f in ["profile-state.db", "profile-state.db-wal"] {
            std::fs::set_permissions(dir.join(f), std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        let index_before = fingerprint(&data);
        let before = fingerprint(&dir);
        assert!(before.iter().any(|f| f.0 == "profile-state.db-wal"));
        assert!(!before.iter().any(|f| f.0.ends_with("-shm")));

        let snap = read_snapshot_with(&missing_cli(&data), &fast()).unwrap();
        assert!(snap.settings.unwrap().nest_workspaces, "WAL row was read");

        let after = fingerprint(&dir);
        assert_eq!(before, after, "bytes, sizes, mtimes and listing unchanged");
        assert!(
            !after.iter().any(|f| f.0.ends_with("-shm")),
            "no -shm created in Orca's directory"
        );
        assert_eq!(index_before, fingerprint(&data));
    }
}
