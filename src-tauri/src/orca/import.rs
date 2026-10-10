//! Import-from-Orca flow behind the Preferences dialog. Orca state is read
//! through an injected reader (read-only, see `source`); the config is only
//! ever changed through `ConfigStore::update`.

use crate::config::{Config, ConfigStore, Project};
use crate::orca::model::{OrcaSnapshot, Source};
use crate::orca::plan::{apply_import, candidates, ImportCandidate, ImportSelection};
use serde::Serialize;
use std::path::Path;

/// What the import dialog shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportList {
    pub source: Source,
    /// The worktree list was cut off: derived roots may be incomplete.
    pub truncated: bool,
    pub candidates: Vec<ImportCandidate>,
}

/// Where Orca's state comes from (injected; the real one is `source`).
pub type ReadSnapshot<'a> = &'a dyn Fn() -> Result<OrcaSnapshot, String>;

/// Candidates for the dialog. `cfg` is a lock-free read.
pub fn list_candidates(
    cfg: &Config,
    home: &Path,
    read: ReadSnapshot,
) -> Result<ImportList, String> {
    let snap = read()?;
    Ok(ImportList {
        source: snap.source,
        truncated: snap.truncated,
        candidates: candidates(&snap, cfg, home),
    })
}

/// Imports the chosen repos. Orca is read BEFORE the config lock is taken
/// (the CLI can take seconds); the candidates are then recomputed against the
/// config as it is INSIDE `update`, so edits made since the dialog was opened
/// are honored. All-or-nothing: any error leaves the config file untouched.
pub fn import_repos(
    store: &ConfigStore,
    home: &Path,
    selections: &[ImportSelection],
    read: ReadSnapshot,
) -> Result<Vec<Project>, String> {
    if selections.is_empty() {
        return Ok(Vec::new());
    }
    let snap = read()?;
    store.update(|cfg| {
        let fresh = candidates(&snap, cfg, home);
        apply_import(cfg, &fresh, selections)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Account, Config, ConfigStore, Project};
    use crate::orca::model::{OrcaRepo, OrcaSettings, OrcaSnapshot, Source};
    use crate::orca::plan::{CandidateStatus, ImportSelection};
    use std::path::Path;

    struct World {
        _dir: tempfile::TempDir,
        home: std::path::PathBuf,
        store: ConfigStore,
    }

    fn account(id: &str) -> Account {
        Account {
            id: id.into(),
            label: id.to_uppercase(),
            config_dir: format!("~/.claude-{id}"),
            inherit_overrides: Default::default(),
            usage_limits: Default::default(),
        }
    }

    fn project(id: &str, path: &str) -> Project {
        Project {
            id: id.into(),
            label: format!("label-{id}"),
            path: path.into(),
            account: "work".into(),
            worktree_roots: vec![],
        }
    }

    /// A temp HOME with real repo folders (`code/<name>`) and a saved config.
    fn world(repos: &[&str], projects: Vec<Project>) -> World {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        for r in repos {
            std::fs::create_dir_all(home.join("code").join(r)).unwrap();
        }
        let cfg = Config {
            terminal: "t".into(),
            accounts: vec![account("work"), account("home")],
            projects,
            tracking: Default::default(),
        };
        let path = dir.path().join("config.json");
        cfg.save(&path).unwrap();
        World {
            home,
            store: ConfigStore::new(path),
            _dir: dir,
        }
    }

    fn snapshot(home: &Path, repos: &[&str]) -> OrcaSnapshot {
        OrcaSnapshot {
            source: Source::Cli,
            repos: repos
                .iter()
                .enumerate()
                .map(|(i, name)| OrcaRepo {
                    id: format!("r{}", i + 1),
                    path: home.join("code").join(name).to_string_lossy().into_owned(),
                    display_name: (*name).into(),
                })
                .collect(),
            worktrees: vec![],
            truncated: true,
            settings: Some(OrcaSettings::default()),
            other_profiles: 0,
            runtime_ready: true,
        }
    }

    fn sel(repo: &str, account: &str) -> ImportSelection {
        ImportSelection {
            orca_repo_id: repo.into(),
            account_id: account.into(),
        }
    }

    fn config_bytes(w: &World) -> Vec<u8> {
        std::fs::read(w.store.path()).unwrap()
    }

    #[test]
    fn test_should_list_candidates_with_source_and_truncated_flag() {
        let w = world(&["repo-a", "repo-b"], vec![project("p1", "~/code/repo-a")]);
        let snap = snapshot(&w.home, &["repo-a", "repo-b"]);
        let cfg = w.store.load().unwrap();
        let list = list_candidates(&cfg, &w.home, &|| Ok(snap.clone())).unwrap();
        assert_eq!(list.source, Source::Cli);
        assert!(list.truncated);
        assert_eq!(list.candidates.len(), 2);
        assert_eq!(list.candidates[0].label, "repo-a");
        assert!(matches!(
            list.candidates[0].status,
            CandidateStatus::AlreadyMapped { .. }
        ));
        assert_eq!(list.candidates[1].status, CandidateStatus::New);
        assert_eq!(list.candidates[1].path, "~/code/repo-b");
    }

    #[test]
    fn test_should_serialize_list_for_the_ui_with_lowercase_source() {
        let w = world(&["repo-a"], vec![]);
        let snap = snapshot(&w.home, &["repo-a"]);
        let cfg = w.store.load().unwrap();
        let list = list_candidates(&cfg, &w.home, &|| Ok(snap.clone())).unwrap();
        let v = serde_json::to_value(&list).unwrap();
        assert_eq!(v["source"], "cli");
        assert_eq!(v["truncated"], true);
        assert_eq!(v["candidates"][0]["status"]["kind"], "new");
    }

    #[test]
    fn test_should_propagate_error_when_orca_state_is_unavailable() {
        let w = world(&[], vec![]);
        let cfg = w.store.load().unwrap();
        let err = list_candidates(&cfg, &w.home, &|| Err("Orca state unavailable: x".into()))
            .unwrap_err();
        assert!(err.contains("unavailable"), "{err}");
        let err = import_repos(&w.store, &w.home, &[sel("r1", "work")], &|| {
            Err("Orca state unavailable: x".into())
        })
        .unwrap_err();
        assert!(err.contains("unavailable"), "{err}");
    }

    #[test]
    fn test_should_import_selected_repos_and_return_projects_when_valid() {
        let w = world(&["repo-a", "repo-b"], vec![project("p5", "~/code/old")]);
        let snap = snapshot(&w.home, &["repo-a", "repo-b"]);
        let added = import_repos(
            &w.store,
            &w.home,
            &[sel("r1", "work"), sel("r2", "home")],
            &|| Ok(snap.clone()),
        )
        .unwrap();
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].id, "p6");
        assert_eq!(added[0].path, "~/code/repo-a");
        assert_eq!(added[1].account, "home");
        let saved = w.store.load().unwrap();
        assert_eq!(saved.projects.len(), 3);
        assert_eq!(saved.project("p7").unwrap().label, "repo-b");
    }

    #[test]
    fn test_should_reject_unknown_account_and_write_nothing() {
        let w = world(&["repo-a", "repo-b"], vec![]);
        let snap = snapshot(&w.home, &["repo-a", "repo-b"]);
        let before = config_bytes(&w);
        let err = import_repos(
            &w.store,
            &w.home,
            &[sel("r1", "work"), sel("r2", "ghost")],
            &|| Ok(snap.clone()),
        )
        .unwrap_err();
        assert!(err.contains("ghost"), "{err}");
        assert_eq!(config_bytes(&w), before, "atomic: nothing written");
    }

    #[test]
    fn test_should_recheck_under_the_lock_when_config_changed_after_listing() {
        let w = world(&["repo-a"], vec![]);
        let snap = snapshot(&w.home, &["repo-a"]);
        // While Orca is being read (no lock held), someone maps the same repo.
        // The import must see that fresh config, not the one from the dialog.
        let err = import_repos(&w.store, &w.home, &[sel("r1", "work")], &|| {
            w.store
                .update(|c| {
                    c.projects.push(project("p1", "~/code/repo-a"));
                    Ok(())
                })
                .unwrap();
            Ok(snap.clone())
        })
        .unwrap_err();
        assert!(err.contains("already covered"), "{err}");
        let saved = w.store.load().unwrap();
        assert_eq!(saved.projects.len(), 1, "only the concurrent edit remains");
    }

    #[test]
    fn test_should_reject_repo_that_vanished_from_orca_when_importing() {
        let w = world(&["repo-a"], vec![]);
        let snap = snapshot(&w.home, &["repo-a"]);
        let before = config_bytes(&w);
        assert!(import_repos(
            &w.store,
            &w.home,
            &[sel("r9", "work")],
            &|| Ok(snap.clone())
        )
        .is_err());
        assert_eq!(config_bytes(&w), before);
    }

    #[test]
    fn test_should_do_nothing_when_nothing_is_selected() {
        let w = world(&["repo-a"], vec![]);
        let before = config_bytes(&w);
        let added = import_repos(&w.store, &w.home, &[], &|| {
            panic!("Orca must not even be read")
        })
        .unwrap();
        assert!(added.is_empty());
        assert_eq!(config_bytes(&w), before);
    }
}
