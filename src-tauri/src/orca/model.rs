//! Orca data model and tolerant parsers.
//!
//! Orca is a moving target we do not control, so every parser reads only the
//! fields it needs, ignores unknown ones, and never panics on odd input. The
//! CLI wraps every `--json` answer in `{ ok, result }`; the SQLite fallback
//! stores one JSON document per domain.

use serde_json::Value;

/// An Orca repository (a project the user added to Orca).
#[derive(Debug, Clone, PartialEq)]
pub struct OrcaRepo {
    pub id: String,
    pub path: String,
    pub display_name: String,
}

/// A worktree Orca knows about, tied to its repo by id.
#[derive(Debug, Clone, PartialEq)]
pub struct OrcaWorktree {
    pub repo_id: String,
    pub path: String,
    pub is_main: bool,
    pub is_archived: bool,
}

/// The slice of Orca's settings we care about. Always read from SQLite: the
/// CLI exposes no settings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OrcaSettings {
    pub workspace_dir: Option<String>,
    pub nest_workspaces: bool,
    /// `agentCmdOverrides.claude`: the command Orca runs for Claude.
    pub claude_cmd_override: Option<String>,
    /// `agentDefaultArgs.claude`: reported verbatim, never modified.
    pub claude_default_args: Option<String>,
    pub managed_accounts: usize,
    pub active_managed: bool,
}

/// Where a snapshot came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cli,
    Sqlite,
}

/// Everything we read from Orca in one go.
#[derive(Debug, Clone, PartialEq)]
pub struct OrcaSnapshot {
    pub source: Source,
    pub repos: Vec<OrcaRepo>,
    pub worktrees: Vec<OrcaWorktree>,
    /// The worktree list hit the request cap: some worktrees are missing.
    pub truncated: bool,
    /// `None` when the settings domain is missing or has an unsupported schema.
    pub settings: Option<OrcaSettings>,
    /// Profiles other than the active one (only noted, never read).
    pub other_profiles: usize,
    pub runtime_ready: bool,
}

/// Unwraps a CLI envelope: requires `ok == true` and a `result`.
pub fn parse_envelope(raw: &str) -> Result<Value, String> {
    let mut v: Value = serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;
    match v.get("ok") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => {
            let msg = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Orca returned ok:false");
            return Err(msg.to_string());
        }
        _ => return Err("not an Orca envelope (missing boolean `ok`)".into()),
    }
    v.get_mut("result")
        .map(Value::take)
        .ok_or_else(|| "envelope has no `result`".to_string())
}

/// Ready iff the runtime reports `state == "ready"` and is reachable.
pub fn parse_status(raw: &str) -> Result<bool, String> {
    let result = parse_envelope(raw)?;
    let runtime = result.get("runtime");
    let state_ready = runtime.and_then(|r| r.get("state")).and_then(Value::as_str) == Some("ready");
    let reachable = runtime
        .and_then(|r| r.get("reachable"))
        .and_then(Value::as_bool)
        == Some(true);
    Ok(state_ready && reachable)
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn base_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn repo_from(v: &Value) -> Option<OrcaRepo> {
    let id = str_field(v, "id")?;
    let path = str_field(v, "path")?;
    let display_name = str_field(v, "displayName").unwrap_or_else(|| base_name(&path));
    Some(OrcaRepo {
        id,
        path,
        display_name,
    })
}

fn repos_from(arr: &Value) -> Vec<OrcaRepo> {
    arr.as_array()
        .map(|a| a.iter().filter_map(repo_from).collect())
        .unwrap_or_default()
}

/// `repo list --json`.
pub fn parse_repo_list(raw: &str) -> Result<Vec<OrcaRepo>, String> {
    let result = parse_envelope(raw)?;
    let repos = result.get("repos").ok_or("result has no `repos`")?;
    Ok(repos_from(repos))
}

/// `worktree list --json`: the worktrees plus whether the list was truncated.
pub fn parse_worktree_list(raw: &str) -> Result<(Vec<OrcaWorktree>, bool), String> {
    let result = parse_envelope(raw)?;
    let list = result
        .get("worktrees")
        .and_then(Value::as_array)
        .ok_or("result has no `worktrees`")?;
    let worktrees = list
        .iter()
        .filter_map(|w| {
            Some(OrcaWorktree {
                repo_id: str_field(w, "repoId")?,
                path: str_field(w, "path")?,
                is_main: w.get("isMainWorktree").and_then(Value::as_bool) == Some(true),
                is_archived: w.get("isArchived").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect();
    let truncated = result.get("truncated").and_then(Value::as_bool) == Some(true);
    Ok((worktrees, truncated))
}

/// SQLite `repos` domain: a top-level JSON array.
pub fn parse_repos_payload(raw: &str) -> Result<Vec<OrcaRepo>, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;
    if !v.is_array() {
        return Err("`repos` payload is not an array".into());
    }
    Ok(repos_from(&v))
}

/// SQLite `worktreeMeta` domain: an object keyed `<repoId>::<path>`. Only the
/// first `::` separates the id (paths may contain `::`). The metadata has no
/// main/archived flags, so those are left false for the caller to derive.
pub fn parse_worktree_meta_payload(raw: &str) -> Result<Vec<OrcaWorktree>, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;
    let map = v
        .as_object()
        .ok_or("`worktreeMeta` payload is not an object")?;
    Ok(map
        .keys()
        .filter_map(|key| {
            let (repo_id, path) = key.split_once("::")?;
            if repo_id.is_empty() || path.is_empty() {
                return None;
            }
            Some(OrcaWorktree {
                repo_id: repo_id.to_string(),
                path: path.to_string(),
                is_main: false,
                is_archived: false,
            })
        })
        .collect())
}

/// SQLite `settings` domain.
pub fn parse_settings_payload(raw: &str) -> Result<OrcaSettings, String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;
    if !v.is_object() {
        return Err("`settings` payload is not an object".into());
    }
    let nested = |outer: &str| {
        v.get(outer)
            .and_then(|o| o.get("claude"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    Ok(OrcaSettings {
        workspace_dir: str_field(&v, "workspaceDir").filter(|s| !s.is_empty()),
        nest_workspaces: v.get("nestWorkspaces").and_then(Value::as_bool) == Some(true),
        claude_cmd_override: nested("agentCmdOverrides"),
        claude_default_args: nested("agentDefaultArgs"),
        managed_accounts: v
            .get("claudeManagedAccounts")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        active_managed: v
            .get("activeClaudeManagedAccountId")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
    })
}

/// `orca-profile-index.json`: the active profile id (defaulting to
/// `local-default`) and how many profiles exist. The id becomes a path
/// component, so anything but a plain single component is rejected.
pub fn parse_profile_index(raw: &str) -> Result<(String, usize), String> {
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;
    let id = str_field(&v, "activeProfileId").unwrap_or_else(|| "local-default".to_string());
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\', '\0']) {
        return Err(format!("unsafe profile id: {id:?}"));
    }
    let count = v
        .get("profiles")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Ok((id, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orca::fixtures::*;

    #[test]
    fn test_should_return_result_when_envelope_ok() {
        let v = parse_envelope(STATUS_READY).unwrap();
        assert_eq!(v["runtime"]["state"], "ready");
    }

    #[test]
    fn test_should_err_when_ok_false_or_garbage() {
        assert!(parse_envelope(ENVELOPE_NOT_OK)
            .unwrap_err()
            .contains("Orca is not running"));
        assert!(parse_envelope("not json").is_err());
        assert!(parse_envelope("").is_err());
        assert!(parse_envelope(r#"{"ok":true}"#).is_err());
        assert!(parse_envelope(r#"{"ok":"yes","result":{}}"#).is_err());
    }

    #[test]
    fn test_should_be_ready_when_state_ready_and_reachable() {
        assert!(parse_status(STATUS_READY).unwrap());
    }

    #[test]
    fn test_should_be_not_ready_when_starting_or_unreachable() {
        assert!(!parse_status(STATUS_STARTING).unwrap());
        assert!(!parse_status(STATUS_UNREACHABLE).unwrap());
        assert!(parse_status(ENVELOPE_NOT_OK).is_err());
    }

    #[test]
    fn test_should_parse_repos_and_worktrees_when_synthetic_json() {
        let repos = parse_repo_list(REPOS_JSON).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].id, "r1");
        assert_eq!(repos[0].path, "/Users/test/code/repo-a");
        assert_eq!(repos[0].display_name, "repo-a");

        let (wts, truncated) = parse_worktree_list(WORKTREES_JSON).unwrap();
        assert!(!truncated);
        assert_eq!(wts.len(), 3);
        assert_eq!(wts[0].repo_id, "r1");
        assert!(wts[0].is_main && !wts[0].is_archived);
        assert!(!wts[1].is_main);
        assert!(wts[2].is_archived);
        assert_eq!(wts[1].path, "/Users/test/orca/workspaces/repo-a/feat-x");
    }

    #[test]
    fn test_should_flag_truncated_when_truncated_true() {
        let (wts, truncated) = parse_worktree_list(WORKTREES_TRUNCATED_JSON).unwrap();
        assert!(truncated);
        assert_eq!(wts.len(), 1);
    }

    #[test]
    fn test_should_skip_repo_when_required_field_missing() {
        let json = r#"{"ok":true,"result":{"repos":[{"id":"r1"},{"id":"r2","path":"/a","displayName":"a"}]}}"#;
        let repos = parse_repo_list(json).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].id, "r2");
    }

    #[test]
    fn test_should_parse_repos_payload_when_top_level_array() {
        let repos = parse_repos_payload(REPOS_PAYLOAD).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[1].display_name, "repo-b");
        assert!(parse_repos_payload(r#"{"not":"an array"}"#).is_err());
        assert!(parse_repos_payload("garbage").is_err());
    }

    #[test]
    fn test_should_split_worktree_meta_key_on_first_separator() {
        let wts = parse_worktree_meta_payload(WORKTREE_META_PAYLOAD).unwrap();
        assert_eq!(wts.len(), 2);
        let w = wts.iter().find(|w| w.repo_id == "r2").unwrap();
        assert_eq!(w.path, "/Users/test/orca/workspaces/repo-b/with::colons");
        assert!(!w.is_main && !w.is_archived);
        // A key without a separator carries no repo: dropped.
        let none = parse_worktree_meta_payload(r#"{"no-separator":{}}"#).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_should_parse_settings_when_payload_has_unknown_fields() {
        let s = parse_settings_payload(SETTINGS_PAYLOAD).unwrap();
        assert_eq!(
            s.workspace_dir.as_deref(),
            Some("/Users/test/orca/workspaces")
        );
        assert!(s.nest_workspaces);
        assert_eq!(s.claude_cmd_override.as_deref(), Some("cms --auto"));
        assert_eq!(s.claude_default_args.as_deref(), Some("--some-flag"));
        assert_eq!(s.managed_accounts, 2);
        assert!(s.active_managed);
    }

    #[test]
    fn test_should_default_settings_when_fields_absent_or_null() {
        let s = parse_settings_payload(r#"{"activeClaudeManagedAccountId":null}"#).unwrap();
        assert_eq!(s, OrcaSettings::default());
        assert!(parse_settings_payload("[]").is_err());
    }

    #[test]
    fn test_should_parse_profile_index_when_active_profile_set() {
        assert_eq!(
            parse_profile_index(PROFILE_INDEX).unwrap(),
            ("p2".to_string(), 3)
        );
    }

    #[test]
    fn test_should_default_profile_when_active_missing() {
        assert_eq!(
            parse_profile_index(r#"{"profiles":[]}"#).unwrap(),
            ("local-default".to_string(), 0)
        );
    }

    #[test]
    fn test_should_reject_profile_id_when_not_a_single_path_component() {
        for bad in ["../x", "a/b", "", ".", "..", "a\\b"] {
            let json = format!(
                r#"{{"activeProfileId":{},"profiles":[]}}"#,
                serde_json::json!(bad)
            );
            assert!(
                parse_profile_index(&json).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }
}
