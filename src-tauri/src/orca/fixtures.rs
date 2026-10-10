//! Synthetic test fixtures only: no real project names or paths. Shapes
//! mirror what the Orca CLI / profile database emit, trimmed to the fields we
//! read plus a few unknown ones (which parsers must ignore).

pub(crate) const STATUS_READY: &str = r#"{"_meta":{"runtimeId":"rt-1"},"id":"x","ok":true,"result":{"app":{"running":true},"runtime":{"state":"ready","reachable":true,"appVersion":"0.0.0"}}}"#;
pub(crate) const STATUS_STARTING: &str =
    r#"{"id":"x","ok":true,"result":{"runtime":{"state":"starting","reachable":true}}}"#;
pub(crate) const STATUS_UNREACHABLE: &str =
    r#"{"id":"x","ok":true,"result":{"runtime":{"state":"ready","reachable":false}}}"#;

pub(crate) const REPOS_JSON: &str = r#"{"_meta":{"runtimeId":"rt-1"},"id":"x","ok":true,"result":{"repos":[
  {"id":"r1","path":"/Users/test/code/repo-a","displayName":"repo-a","kind":"git","badgeColor":"fff","extra":{"k":1}},
  {"id":"r2","path":"/Users/test/code/repo-b","displayName":"repo-b","kind":"git"}
]}}"#;

pub(crate) const WORKTREES_JSON: &str = r#"{"_meta":{"runtimeId":"rt-1"},"id":"x","ok":true,"result":{"totalCount":3,"truncated":false,"worktrees":[
  {"id":"w1","path":"/Users/test/code/repo-a","repoId":"r1","isMainWorktree":true,"isArchived":false,"displayName":"main","branch":"refs/heads/main"},
  {"id":"w2","path":"/Users/test/orca/workspaces/repo-a/feat-x","repoId":"r1","isMainWorktree":false,"isArchived":false,"displayName":"feat-x"},
  {"id":"w3","path":"/Users/test/orca/workspaces/repo-b/old","repoId":"r2","isMainWorktree":false,"isArchived":true,"displayName":"old"}
]}}"#;

pub(crate) const WORKTREES_TRUNCATED_JSON: &str = r#"{"id":"x","ok":true,"result":{"totalCount":9,"truncated":true,"worktrees":[
  {"id":"w1","path":"/Users/test/code/repo-a","repoId":"r1","isMainWorktree":true,"isArchived":false}
]}}"#;

pub(crate) const ENVELOPE_NOT_OK: &str = r#"{"id":"x","ok":false,"error":{"code":"runtime_unavailable","message":"Orca is not running"}}"#;

/// SQLite `repos` domain payload: a top-level array.
pub(crate) const REPOS_PAYLOAD: &str = r#"[
  {"id":"r1","path":"/Users/test/code/repo-a","displayName":"repo-a","kind":"git","hookSettings":{"mode":"x"}},
  {"id":"r2","path":"/Users/test/code/repo-b","displayName":"repo-b"}
]"#;

/// SQLite `worktreeMeta` domain payload: object keyed `<repoId>::<path>`.
pub(crate) const WORKTREE_META_PAYLOAD: &str = r#"{
  "r1::/Users/test/orca/workspaces/repo-a/feat-x":{"displayName":"feat-x","workspaceStatus":"in-progress"},
  "r2::/Users/test/orca/workspaces/repo-b/with::colons":{"displayName":"y"}
}"#;

/// SQLite `settings` domain payload.
pub(crate) const SETTINGS_PAYLOAD: &str = r#"{
  "workspaceDir":"/Users/test/orca/workspaces",
  "nestWorkspaces":true,
  "agentCmdOverrides":{"claude":"cms --auto"},
  "agentDefaultArgs":{"claude":"--some-flag","codex":"--other"},
  "claudeManagedAccounts":[{"id":"m1"},{"id":"m2"}],
  "activeClaudeManagedAccountId":"m1",
  "unrelated":{"deep":[1,2,3]}
}"#;

pub(crate) const PROFILE_INDEX: &str = r#"{"schemaVersion":1,"activeProfileId":"p2","profiles":[{"id":"p1","name":"One"},{"id":"p2","name":"Two"},{"id":"p3","name":"Three"}]}"#;

/// A fake `orca` CLI: a temp `#!/bin/sh` script next to the JSON files it
/// prints. Every invocation's argv is appended to `calls.log`.
#[cfg(target_os = "macos")]
pub(crate) struct FakeCli {
    pub dir: tempfile::TempDir,
    pub path: std::path::PathBuf,
}

#[cfg(target_os = "macos")]
impl FakeCli {
    /// The standard script: `status`, `repo list` and `worktree list` print
    /// `status.json`, `repos.json` and `worktrees.json` (or
    /// `worktrees_small.json` when the requested limit is 500 and that file
    /// exists). Anything else exits 3.
    pub fn standard() -> Self {
        Self::with_script(
            r#"case "$1 $2" in
"status --json") cat "$D/status.json";;
"repo list") cat "$D/repos.json";;
"worktree list")
  if [ "$5" = "500" ] && [ -f "$D/worktrees_small.json" ]; then cat "$D/worktrees_small.json"; else cat "$D/worktrees.json"; fi;;
*) echo "unexpected: $*" >&2; exit 3;;
esac"#,
        )
    }

    pub fn with_script(body: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orca");
        let script =
            format!("#!/bin/sh\nD=$(dirname \"$0\")\necho \"$@\" >> \"$D/calls.log\"\n{body}\n");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        FakeCli { dir, path }
    }

    /// Standard script with the ready status and the synthetic repo/worktree JSON.
    pub fn healthy() -> Self {
        let f = Self::standard();
        f.put("status.json", STATUS_READY);
        f.put("repos.json", REPOS_JSON);
        f.put("worktrees.json", WORKTREES_JSON);
        f
    }

    pub fn put(&self, name: &str, content: &str) {
        std::fs::write(self.dir.path().join(name), content).unwrap();
    }

    /// Recorded argv lines, in call order.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// Writes `orca-profile-index.json` (synthetic) into `data_dir`.
#[cfg(target_os = "macos")]
pub(crate) fn write_profile_index(data_dir: &std::path::Path, active: &str, ids: &[&str]) {
    std::fs::create_dir_all(data_dir).unwrap();
    let profiles: Vec<_> = ids
        .iter()
        .map(|id| serde_json::json!({"id": id, "name": id}))
        .collect();
    let index =
        serde_json::json!({"schemaVersion": 1, "activeProfileId": active, "profiles": profiles});
    std::fs::write(
        data_dir.join("orca-profile-index.json"),
        serde_json::to_string(&index).unwrap(),
    )
    .unwrap();
}

/// Builds a synthetic profile database the way a live Orca leaves it: WAL
/// mode, with `wal_updates` applied AFTER a checkpoint so they exist only in
/// the `-wal` file. The files are copied to
/// `<data_dir>/profiles/<profile_id>/` while the writer is still open, then
/// the writer is dropped, so the target holds db + non-empty wal and no `-shm`.
///
/// `docs` are `(domain, payload, domain_version)`.
#[cfg(target_os = "macos")]
pub(crate) fn write_profile_db(
    data_dir: &std::path::Path,
    profile_id: &str,
    docs: &[(&str, &str, i64)],
    wal_updates: &[(&str, &str)],
) -> std::path::PathBuf {
    let staging = tempfile::tempdir().unwrap();
    let db = staging.path().join("profile-state.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    conn.execute_batch(
        "CREATE TABLE profile_state_meta(key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE profile_state_documents(
           domain TEXT PRIMARY KEY, payload TEXT NOT NULL, domain_version INTEGER NOT NULL,
           revision INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0,
           content_hash TEXT NOT NULL DEFAULT '');",
    )
    .unwrap();
    for (domain, payload, version) in docs {
        conn.execute(
            "INSERT INTO profile_state_documents(domain, payload, domain_version) VALUES (?1, ?2, ?3)",
            rusqlite::params![domain, payload, version],
        )
        .unwrap();
    }
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    for (domain, payload) in wal_updates {
        conn.execute(
            "UPDATE profile_state_documents SET payload = ?2 WHERE domain = ?1",
            rusqlite::params![domain, payload],
        )
        .unwrap();
    }
    let target = data_dir.join("profiles").join(profile_id);
    std::fs::create_dir_all(&target).unwrap();
    std::fs::copy(&db, target.join("profile-state.db")).unwrap();
    let wal = staging.path().join("profile-state.db-wal");
    if wal.exists() {
        std::fs::copy(&wal, target.join("profile-state.db-wal")).unwrap();
    }
    drop(conn);
    target
}

/// The three domains Orca integration reads, all at schema version 1.
#[cfg(target_os = "macos")]
pub(crate) fn standard_docs<'a>(
    repos: &'a str,
    meta: &'a str,
    settings: &'a str,
) -> Vec<(&'static str, &'a str, i64)> {
    vec![
        ("repos", repos, 1),
        ("worktreeMeta", meta, 1),
        ("settings", settings, 1),
    ]
}
