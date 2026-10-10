//! Pure derivation of "worktree roots" for an Orca repo: the folders under
//! which Orca creates (or the user keeps) that repo's worktrees, so
//! `cms --auto` can map a working directory back to the project.

use crate::cli::normalize_lexical;
use crate::orca::model::{OrcaRepo, OrcaSettings, OrcaWorktree};
use std::path::{Path, PathBuf};

/// Absolute roots that are scratch space: a worktree parked there says
/// nothing about where the project's worktrees live.
const TMP_ROOTS: &[&str] = &[
    "/tmp",
    "/private/tmp",
    "/var/folders",
    "/private/var/folders",
];

/// Canonical form used for comparisons: symlinks resolved when the path
/// exists, plain lexical normalization otherwise (never an error).
pub fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| normalize_lexical(p))
}

/// Derives the worktree roots of `repo` (tilde-collapsed against `home`).
pub fn worktree_roots(
    repo: &OrcaRepo,
    worktrees: &[OrcaWorktree],
    settings: Option<&OrcaSettings>,
    home: &Path,
) -> Vec<String> {
    let mut tmp: Vec<PathBuf> = TMP_ROOTS.iter().map(PathBuf::from).collect();
    if let Some(t) = std::env::var_os("TMPDIR").filter(|t| !t.is_empty()) {
        tmp.push(PathBuf::from(t));
    }
    worktree_roots_excluding(repo, worktrees, settings, home, &tmp)
}

/// Same as [`worktree_roots`] with an explicit list of scratch roots to skip.
pub fn worktree_roots_excluding(
    repo: &OrcaRepo,
    worktrees: &[OrcaWorktree],
    settings: Option<&OrcaSettings>,
    home: &Path,
    tmp_roots: &[PathBuf],
) -> Vec<String> {
    let repo_path = Path::new(&repo.path);
    let repo_canon = canon(repo_path);
    let home_canon = canon(home);
    let mut found: Vec<PathBuf> = Vec::new();

    // (1) Where Orca nests new worktrees; kept even if it does not exist yet.
    if let Some(s) = settings.filter(|s| s.nest_workspaces) {
        if let Some(dir) = s.workspace_dir.as_deref().filter(|d| !d.is_empty()) {
            found.push(Path::new(dir).join(&repo.display_name));
        }
    }

    // (2) Parents of this repo's live, non-main worktrees.
    for w in worktrees
        .iter()
        .filter(|w| w.repo_id == repo.id && !w.is_main && !w.is_archived)
    {
        let Some(parent) = Path::new(&w.path).parent() else {
            continue;
        };
        let parent_lex = normalize_lexical(parent);
        let parent_canon = canon(parent);
        let scratch = tmp_roots.iter().any(|t| {
            parent_lex.starts_with(normalize_lexical(t)) || parent_canon.starts_with(canon(t))
        });
        let too_broad = parent_canon.parent().is_none() // `/`
            || home_canon.starts_with(&parent_canon) // HOME or one of its ancestors
            || repo_canon.starts_with(&parent_canon); // contains the repo itself
        let covered = parent_canon.starts_with(&repo_canon); // the project path already covers it
        if scratch || too_broad || covered {
            continue;
        }
        found.push(parent_lex);
    }

    // (3) Dedupe by canonical form; an ancestor swallows its descendants.
    let mut kept: Vec<(PathBuf, PathBuf)> = Vec::new(); // (as emitted, canonical)
    for p in found {
        let p = normalize_lexical(&p);
        let c = canon(&p);
        if kept.iter().any(|(_, kc)| c.starts_with(kc)) {
            continue;
        }
        kept.retain(|(_, kc)| !kc.starts_with(&c));
        kept.push((p, c));
    }

    kept.into_iter().map(|(p, _)| tildify(&p, home)).collect()
}

fn tildify(p: &Path, home: &Path) -> String {
    match p.strip_prefix(normalize_lexical(home)) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => p.to_string_lossy().into_owned(),
    }
}

// Orca integration is macOS-only; these tests assert POSIX paths.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::orca::model::{OrcaRepo, OrcaSettings, OrcaWorktree};
    use std::path::Path;

    const HOME: &str = "/Users/test";

    fn repo(path: &str, name: &str) -> OrcaRepo {
        OrcaRepo {
            id: "r1".into(),
            path: path.into(),
            display_name: name.into(),
        }
    }

    fn wt(path: &str) -> OrcaWorktree {
        OrcaWorktree {
            repo_id: "r1".into(),
            path: path.into(),
            is_main: false,
            is_archived: false,
        }
    }

    fn settings(nest: bool) -> OrcaSettings {
        OrcaSettings {
            workspace_dir: Some("/Users/test/orca/workspaces".into()),
            nest_workspaces: nest,
            ..OrcaSettings::default()
        }
    }

    fn roots(r: &OrcaRepo, w: &[OrcaWorktree], s: Option<&OrcaSettings>) -> Vec<String> {
        worktree_roots(r, w, s, Path::new(HOME))
    }

    #[test]
    fn test_should_include_workspace_dir_root_when_nest_true() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        assert_eq!(
            roots(&r, &[], Some(&settings(true))),
            vec!["~/orca/workspaces/repo-a"]
        );
    }

    #[test]
    fn test_should_omit_workspace_dir_root_when_nest_false() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        assert!(roots(&r, &[], Some(&settings(false))).is_empty());
        assert!(roots(&r, &[], None).is_empty());
    }

    #[test]
    fn test_should_exclude_archived_main_repo_inside_and_tmp_worktrees() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let mut archived = wt("/Users/test/old/archived/x");
        archived.is_archived = true;
        let mut main = wt("/Users/test/elsewhere/main-copy");
        main.is_main = true;
        let inside_repo = wt("/Users/test/code/repo-a/.worktrees/x");
        let tmp1 = wt("/private/tmp/scratch/x");
        let tmp2 = wt("/var/folders/ab/cd/T/x");
        let tmp3 = wt("/tmp/x");
        let ok = wt("/Users/test/code/repo-a-wt/feat");
        let got = roots(
            &r,
            &[archived, main, inside_repo, tmp1, tmp2, tmp3, ok],
            None,
        );
        assert_eq!(got, vec!["~/code/repo-a-wt"]);
    }

    #[test]
    fn test_should_exclude_worktrees_of_other_repos() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let mut other = wt("/Users/test/other/x");
        other.repo_id = "r2".into();
        assert!(roots(&r, &[other], None).is_empty());
    }

    #[test]
    fn test_should_exclude_parents_that_are_home_root_or_contain_the_repo() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let got = roots(
            &r,
            &[
                wt("/Users/test/direct"),      // parent == HOME
                wt("/Users/direct"),           // parent is an ancestor of HOME
                wt("/direct"),                 // parent == /
                wt("/Users/test/code/feat-b"), // parent contains the repo itself
            ],
            None,
        );
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn test_should_emit_one_root_when_worktrees_share_a_parent() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let got = roots(
            &r,
            &[
                wt("/Users/test/wts/repo-a/one"),
                wt("/Users/test/wts/repo-a/two"),
            ],
            None,
        );
        assert_eq!(got, vec!["~/wts/repo-a"]);
    }

    #[test]
    fn test_should_keep_ancestor_when_roots_nest() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let got = roots(
            &r,
            &[
                wt("/Users/test/wts/a/deep/one"),
                wt("/Users/test/wts/a/two"),
            ],
            None,
        );
        assert_eq!(got, vec!["~/wts/a"]);
    }

    #[test]
    fn test_should_dedupe_workspace_root_and_worktree_parent() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let got = roots(
            &r,
            &[wt("/Users/test/orca/workspaces/repo-a/feat-x")],
            Some(&settings(true)),
        );
        assert_eq!(got, vec!["~/orca/workspaces/repo-a"]);
    }

    #[test]
    fn test_should_keep_absolute_path_when_outside_home() {
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let got = roots(&r, &[wt("/Volumes/data/wts/x")], None);
        assert_eq!(got, vec!["/Volumes/data/wts"]);
    }

    #[test]
    fn test_should_leave_nonexistent_path_unchanged_when_canonicalizing() {
        let p = Path::new("/Users/test/does/not/./exist/../here");
        assert_eq!(canon(p), PathBuf::from("/Users/test/does/not/here"));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_collapse_symlink_and_nosync_when_canonicalizing() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("Project.nosync");
        std::fs::create_dir_all(real.join("wts")).unwrap();
        let link = tmp.path().join("Project");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(canon(&link.join("wts")), canon(&real.join("wts")));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_dedupe_symlink_forms_when_deriving_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("Project.nosync");
        std::fs::create_dir_all(real.join("wts/repo-a/one")).unwrap();
        let link = tmp.path().join("Project");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let r = repo("/Users/test/code/repo-a", "repo-a");
        let via_link = wt(&link.join("wts/repo-a/one").to_string_lossy());
        let via_real = wt(&real.join("wts/repo-a/two").to_string_lossy());
        // Temp dirs live under a tmp-like root: disable that rule for this test.
        let got = worktree_roots_excluding(&r, &[via_link, via_real], None, Path::new(HOME), &[]);
        assert_eq!(got.len(), 1, "{got:?}");
    }
}
