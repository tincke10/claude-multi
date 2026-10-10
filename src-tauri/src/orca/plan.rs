//! Pure import planning: which Orca repos can become claude-multi projects,
//! and the config mutation that imports the chosen ones. Orca is never
//! touched; this only reads a snapshot and edits our own `Config`.

use crate::config::{Config, Project};
use crate::orca::derive::{canon, worktree_roots};
use crate::orca::model::OrcaSnapshot;
use crate::paths::expand_tilde;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Whether (and why not) a repo can be imported.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CandidateStatus {
    New,
    /// A project (by label) already covers this repo's path.
    AlreadyMapped {
        project: String,
    },
    /// The repo's folder no longer exists on disk.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportCandidate {
    pub orca_repo_id: String,
    pub label: String,
    /// Orca's repo path, verbatim (only tilde-collapsed), so Orca's own path
    /// selectors keep matching it.
    pub path: String,
    pub worktree_roots: Vec<String>,
    pub status: CandidateStatus,
}

/// The user's choice for one candidate.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ImportSelection {
    pub orca_repo_id: String,
    pub account_id: String,
}

fn tildify(path: &str, home: &Path) -> String {
    match Path::new(path).strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string(),
    }
}

/// Canonical form of a stored (possibly `~`-prefixed) path.
fn canon_stored(path: &str) -> PathBuf {
    canon(&expand_tilde(path.trim()))
}

/// Same, expanding `~` against an explicit `home` (deterministic planning).
fn canon_stored_in(path: &str, home: &Path) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix("~/") {
        Some(rest) => canon(&home.join(rest)),
        None => canon(Path::new(path)),
    }
}

/// Candidates for every repo in the snapshot, sorted by label.
pub fn candidates(snapshot: &OrcaSnapshot, cfg: &Config, home: &Path) -> Vec<ImportCandidate> {
    candidates_with(snapshot, cfg, home, &|p| p.is_dir())
}

/// [`candidates`] with an injectable "does this folder exist" check.
pub fn candidates_with(
    snapshot: &OrcaSnapshot,
    cfg: &Config,
    home: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<ImportCandidate> {
    let mut out: Vec<ImportCandidate> = snapshot
        .repos
        .iter()
        .map(|repo| {
            let repo_path = Path::new(&repo.path);
            let repo_canon = canon(repo_path);
            let mapped = cfg.projects.iter().find(|p| {
                !p.path.trim().is_empty() && canon_stored_in(&p.path, home) == repo_canon
                    || p.worktree_roots
                        .iter()
                        .filter(|r| !r.trim().is_empty())
                        .map(|r| canon_stored_in(r, home))
                        .any(|r| repo_canon.starts_with(&r) || r.starts_with(&repo_canon))
            });
            let status = if let Some(p) = mapped {
                CandidateStatus::AlreadyMapped {
                    project: p.label.clone(),
                }
            } else if !exists(repo_path) {
                CandidateStatus::Missing
            } else {
                CandidateStatus::New
            };
            ImportCandidate {
                orca_repo_id: repo.id.clone(),
                label: repo.display_name.clone(),
                path: tildify(&repo.path, home),
                worktree_roots: worktree_roots(
                    repo,
                    &snapshot.worktrees,
                    snapshot.settings.as_ref(),
                    home,
                ),
                status,
            }
        })
        .collect();
    out.sort_by_key(|c| c.label.to_lowercase());
    out
}

fn next_project_ids(cfg: &Config) -> impl FnMut() -> String {
    let mut max = cfg
        .projects
        .iter()
        .filter_map(|p| p.id.strip_prefix('p')?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    move || {
        max += 1;
        format!("p{max}")
    }
}

/// Imports the selected candidates into `cfg`. Every check runs before any
/// mutation, so an `Err` leaves `cfg` untouched (and, inside
/// `ConfigStore::update`, nothing is written). Returns the added projects.
pub fn apply_import(
    cfg: &mut Config,
    candidates: &[ImportCandidate],
    selections: &[ImportSelection],
) -> Result<Vec<Project>, String> {
    // Paths and roots already taken, canonicalized: existing projects first,
    // then each item as it is accepted.
    let mut taken: HashSet<PathBuf> = HashSet::new();
    for p in &cfg.projects {
        if !p.path.trim().is_empty() {
            taken.insert(canon_stored(&p.path));
        }
        for r in p.worktree_roots.iter().filter(|r| !r.trim().is_empty()) {
            taken.insert(canon_stored(r));
        }
    }

    let mut seen_repos: HashSet<&str> = HashSet::new();
    let mut staged: Vec<(&ImportCandidate, &str)> = Vec::new();
    for sel in selections {
        let cand = candidates
            .iter()
            .find(|c| c.orca_repo_id == sel.orca_repo_id)
            .ok_or_else(|| format!("unknown Orca repo '{}'", sel.orca_repo_id))?;
        if !seen_repos.insert(&sel.orca_repo_id) {
            return Err(format!("'{}' was selected more than once", cand.label));
        }
        if cfg.account(&sel.account_id).is_none() {
            return Err(format!(
                "unknown account '{}' for '{}'",
                sel.account_id, cand.label
            ));
        }
        match &cand.status {
            CandidateStatus::New => {}
            CandidateStatus::AlreadyMapped { project } => {
                return Err(format!(
                    "'{}' is already covered by project '{project}'",
                    cand.label
                ))
            }
            CandidateStatus::Missing => {
                return Err(format!("'{}' no longer exists on disk", cand.label))
            }
        }
        let path = canon_stored(&cand.path);
        if !taken.insert(path) {
            return Err(format!(
                "'{}' collides with an existing project path or worktree root",
                cand.label
            ));
        }
        for root in cand.worktree_roots.iter().filter(|r| !r.trim().is_empty()) {
            if !taken.insert(canon_stored(root)) {
                return Err(format!(
                    "worktree root '{root}' of '{}' collides with an existing project path or worktree root",
                    cand.label
                ));
            }
        }
        staged.push((cand, &sel.account_id));
    }

    let mut next_id = next_project_ids(cfg);
    let added: Vec<Project> = staged
        .into_iter()
        .map(|(c, account)| Project {
            id: next_id(),
            label: c.label.clone(),
            path: c.path.clone(),
            account: account.to_string(),
            worktree_roots: c.worktree_roots.clone(),
        })
        .collect();
    cfg.projects.extend(added.iter().cloned());
    Ok(added)
}

// Orca integration is macOS-only; these tests assert POSIX paths.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config::{Account, Config, ConfigStore, Project};
    use crate::orca::model::{OrcaRepo, OrcaSettings, OrcaSnapshot, OrcaWorktree, Source};
    use std::path::Path;

    const HOME: &str = "/Users/test";

    fn repo(id: &str, name: &str) -> OrcaRepo {
        OrcaRepo {
            id: id.into(),
            path: format!("/Users/test/code/{name}"),
            display_name: name.into(),
        }
    }

    fn snapshot(repos: Vec<OrcaRepo>, worktrees: Vec<OrcaWorktree>) -> OrcaSnapshot {
        OrcaSnapshot {
            source: Source::Cli,
            repos,
            worktrees,
            truncated: false,
            settings: Some(OrcaSettings {
                workspace_dir: Some("/Users/test/orca/workspaces".into()),
                nest_workspaces: true,
                ..OrcaSettings::default()
            }),
            other_profiles: 0,
            runtime_ready: true,
        }
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

    fn project(id: &str, path: &str, roots: &[&str]) -> Project {
        Project {
            id: id.into(),
            label: format!("label-{id}"),
            path: path.into(),
            account: "work".into(),
            worktree_roots: roots.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn config(projects: Vec<Project>) -> Config {
        Config {
            terminal: "t".into(),
            accounts: vec![account("work"), account("home")],
            projects,
            tracking: Default::default(),
            usage: Default::default(),
        }
    }

    fn plan(snap: &OrcaSnapshot, cfg: &Config) -> Vec<ImportCandidate> {
        candidates_with(snap, cfg, Path::new(HOME), &|_| true)
    }

    #[test]
    fn test_should_mark_already_mapped_when_path_equals_project_path() {
        let cfg = config(vec![project("p1", "~/code/repo-a", &[])]);
        let got = plan(&snapshot(vec![repo("r1", "repo-a")], vec![]), &cfg);
        assert_eq!(
            got[0].status,
            CandidateStatus::AlreadyMapped {
                project: "label-p1".into()
            }
        );
    }

    #[test]
    fn test_should_mark_already_mapped_when_repo_overlaps_worktree_root() {
        // Repo lies under an existing root...
        let under = config(vec![project("p1", "/elsewhere/x", &["~/code"])]);
        let got = plan(&snapshot(vec![repo("r1", "repo-a")], vec![]), &under);
        assert!(matches!(
            got[0].status,
            CandidateStatus::AlreadyMapped { .. }
        ));
        // ...or over it.
        let over = config(vec![project("p1", "/elsewhere/x", &["~/code/repo-a/wts"])]);
        let got = plan(&snapshot(vec![repo("r1", "repo-a")], vec![]), &over);
        assert!(matches!(
            got[0].status,
            CandidateStatus::AlreadyMapped { .. }
        ));
    }

    #[test]
    fn test_should_offer_candidate_with_derived_roots_when_unmapped() {
        let cfg = config(vec![project("p1", "~/code/other", &[])]);
        let got = plan(&snapshot(vec![repo("r1", "repo-a")], vec![]), &cfg);
        assert_eq!(got.len(), 1);
        let c = &got[0];
        assert_eq!(c.status, CandidateStatus::New);
        assert_eq!(c.orca_repo_id, "r1");
        assert_eq!(c.label, "repo-a");
        assert_eq!(c.path, "~/code/repo-a");
        assert_eq!(c.worktree_roots, vec!["~/orca/workspaces/repo-a"]);
    }

    #[test]
    fn test_should_mark_missing_when_repo_dir_does_not_exist() {
        let snap = snapshot(vec![repo("r1", "repo-a")], vec![]);
        let got = candidates_with(&snap, &config(vec![]), Path::new(HOME), &|_| false);
        assert_eq!(got[0].status, CandidateStatus::Missing);
    }

    #[test]
    fn test_should_sort_candidates_by_label_when_listing() {
        let snap = snapshot(vec![repo("r1", "zeta"), repo("r2", "Alpha")], vec![]);
        let got = plan(&snap, &config(vec![]));
        let labels: Vec<_> = got.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["Alpha", "zeta"]);
    }

    fn sel(repo: &str, account: &str) -> ImportSelection {
        ImportSelection {
            orca_repo_id: repo.into(),
            account_id: account.into(),
        }
    }

    fn new_candidate(id: &str, name: &str, roots: &[&str]) -> ImportCandidate {
        ImportCandidate {
            orca_repo_id: id.into(),
            label: name.into(),
            path: format!("~/code/{name}"),
            worktree_roots: roots.iter().map(|s| s.to_string()).collect(),
            status: CandidateStatus::New,
        }
    }

    #[test]
    fn test_should_err_when_account_is_unknown() {
        let mut cfg = config(vec![]);
        let err = apply_import(
            &mut cfg,
            &[new_candidate("r1", "repo-a", &[])],
            &[sel("r1", "ghost")],
        )
        .unwrap_err();
        assert!(err.contains("ghost"), "{err}");
        assert!(cfg.projects.is_empty());
    }

    #[test]
    fn test_should_err_when_selection_is_unknown_or_not_new() {
        let mut cfg = config(vec![]);
        assert!(apply_import(&mut cfg, &[], &[sel("nope", "work")]).is_err());
        let mut c = new_candidate("r1", "repo-a", &[]);
        c.status = CandidateStatus::Missing;
        assert!(apply_import(&mut cfg, &[c.clone()], &[sel("r1", "work")]).is_err());
        c.status = CandidateStatus::AlreadyMapped {
            project: "x".into(),
        };
        assert!(apply_import(&mut cfg, &[c], &[sel("r1", "work")]).is_err());
        assert!(cfg.projects.is_empty());
    }

    #[test]
    fn test_should_err_when_same_repo_selected_twice() {
        let mut cfg = config(vec![]);
        let c = new_candidate("r1", "repo-a", &[]);
        assert!(apply_import(&mut cfg, &[c], &[sel("r1", "work"), sel("r1", "home")]).is_err());
        assert!(cfg.projects.is_empty());
    }

    fn store(cfg: &Config) -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        cfg.save(&path).unwrap();
        (dir, ConfigStore::new(path))
    }

    #[test]
    fn test_should_save_both_projects_when_two_valid_items() {
        let (_d, store) = store(&config(vec![project("p5", "~/code/old", &[])]));
        let cands = [
            new_candidate("r1", "repo-a", &["~/wts/a"]),
            new_candidate("r2", "repo-b", &[]),
        ];
        let added = store
            .update(|cfg| apply_import(cfg, &cands, &[sel("r1", "work"), sel("r2", "home")]))
            .unwrap();
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].id, "p6");
        assert_eq!(added[1].id, "p7");
        let saved = store.load().unwrap();
        assert_eq!(saved.projects.len(), 3);
        let a = saved.project("p6").unwrap();
        assert_eq!(a.label, "repo-a");
        assert_eq!(a.path, "~/code/repo-a");
        assert_eq!(a.account, "work");
        assert_eq!(a.worktree_roots, vec!["~/wts/a"]);
        assert_eq!(saved.project("p7").unwrap().account, "home");
    }

    #[test]
    fn test_should_err_and_leave_config_unchanged_when_item_collides_with_existing_root() {
        let (d, store) = store(&config(vec![project(
            "p1",
            "~/code/old",
            &["~/wts/shared"],
        )]));
        let before = std::fs::read(d.path().join("config.json")).unwrap();
        let cands = [
            new_candidate("r1", "repo-a", &["~/wts/fine"]),
            new_candidate("r2", "repo-b", &["~/wts/shared"]),
        ];
        let err = store
            .update(|cfg| apply_import(cfg, &cands, &[sel("r1", "work"), sel("r2", "work")]))
            .unwrap_err();
        assert!(err.contains("repo-b"), "{err}");
        assert_eq!(std::fs::read(d.path().join("config.json")).unwrap(), before);
    }

    #[test]
    fn test_should_err_when_two_items_share_a_root_or_path_exists() {
        let mut cfg = config(vec![project("p1", "~/code/repo-a", &[])]);
        let dup_path = new_candidate("r1", "repo-a", &[]);
        assert!(apply_import(&mut cfg, &[dup_path], &[sel("r1", "work")]).is_err());
        let a = new_candidate("r2", "repo-b", &["~/wts/x"]);
        let b = new_candidate("r3", "repo-c", &["~/wts/x"]);
        assert!(apply_import(&mut cfg, &[a, b], &[sel("r2", "work"), sel("r3", "work")]).is_err());
        assert_eq!(cfg.projects.len(), 1);
    }
}
