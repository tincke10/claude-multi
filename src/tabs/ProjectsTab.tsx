import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Config, InheritDecision, InheritSubdirStatus, Project, getInheritStatus, setInheritDecision,
} from "../api";
import { IN_TAURI } from "../hooks/useConfig";
import { nextId, updateAt } from "../lib/configDirs";
import { formatWorktreeRoots, parseWorktreeRoots, worktreeRootsProblem } from "../lib/worktreeRoots";

type Props = {
  config: Config;
  patch: (next: Partial<Config>) => void;
  applyPersisted: (fn: (c: Config) => Config) => void;
};

export default function ProjectsTab({ config: cfg, patch, applyPersisted }: Props) {
  // Inheritance panel: which account is inspected + its fetched per-subdir rows.
  const [inheritSel, setInheritSel] = useState<string>(cfg.accounts[0]?.id ?? "");
  const inheritAccount = cfg.accounts.some((a) => a.id === inheritSel)
    ? inheritSel
    : (cfg.accounts[0]?.id ?? "");
  const [inheritRows, setInheritRows] = useState<InheritSubdirStatus[]>([]);
  const [inheritErr, setInheritErr] = useState<string | null>(null);
  // Raw worktree-roots text per project id: the textarea keeps the raw text (a
  // trailing comma must survive typing) while the config holds the parsed list.
  const [rootsDrafts, setRootsDrafts] = useState<Record<string, string>>({});

  // Auto-refresh the inheritance status whenever the inspected account changes.
  useEffect(() => {
    if (!IN_TAURI || !inheritAccount) {
      setInheritRows([]);
      return;
    }
    let cancelled = false;
    getInheritStatus(inheritAccount)
      .then((rows) => { if (!cancelled) { setInheritRows(rows); setInheritErr(null); } })
      .catch((e) => { if (!cancelled) { setInheritErr(`${e}`); setInheritRows([]); } });
    return () => { cancelled = true; };
  }, [inheritAccount]);

  const setProjects = (projects: Project[]) => patch({ projects });
  const rootsDraft = (p: Project) => rootsDrafts[p.id] ?? formatWorktreeRoots(p.worktree_roots);
  const rootsErrors = cfg.projects.map((p) => worktreeRootsProblem(p.worktree_roots));
  const setRoots = (i: number, raw: string) => {
    setRootsDrafts({ ...rootsDrafts, [cfg.projects[i].id]: raw });
    setProjects(updateAt(cfg.projects, i, { worktree_roots: parseWorktreeRoots(raw) }));
  };

  const addProject = () => {
    const id = nextId(cfg.projects.map((p) => p.id), "p");
    setProjects([...cfg.projects, { id, label: "", path: "", account: cfg.accounts[0]?.id ?? "", worktree_roots: [] }]);
  };

  const browseProject = async (i: number) => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir !== "string") return; // cancelled
    const current = cfg.projects[i];
    const folderName = dir.split(/[\\/]/).pop() || current.label; // handle / and \
    const labelIsDefault = current.label.trim() === "" || /^Project \d+$/.test(current.label);
    setProjects(updateAt(cfg.projects, i, { path: dir, label: labelIsDefault ? folderName : current.label }));
  };

  // Persist a Merge/Skip decision for one subdir, then re-fetch to reflect it.
  const onInheritToggle = async (subdir: string, decision: InheritDecision) => {
    try {
      await setInheritDecision(inheritAccount, subdir, decision);
      // Mirror the persisted decision locally (form AND snapshot) so a later
      // Save doesn't send a stale map and the form isn't marked dirty.
      applyPersisted((c) => ({
        ...c,
        accounts: c.accounts.map((a) =>
          a.id === inheritAccount
            ? { ...a, inherit_overrides: { ...a.inherit_overrides, [subdir]: decision } }
            : a),
      }));
      setInheritRows(await getInheritStatus(inheritAccount));
      setInheritErr(null);
    } catch (e) {
      setInheritErr(`${e}`);
    }
  };

  return (
    <>
      <section className="card">
        <div className="card__head">
          <h2 className="card__title">Projects</h2>
          <button className="btn btn--ghost" onClick={addProject} disabled={cfg.accounts.length === 0}>Add project</button>
        </div>
        <p className="card__hint">Each project belongs to one account and appears only under that account in the tray.</p>

        {cfg.projects.length === 0 && <p className="muted">No projects yet. Add one and pick a folder.</p>}
        {cfg.projects.map((p, i) => (
          <div className="project" key={p.id}>
            <div className="row">
              <input
                className="input row__label"
                placeholder="Label"
                value={p.label}
                onChange={(e) => setProjects(updateAt(cfg.projects, i, { label: e.target.value }))}
              />
              <input
                className="input row__grow"
                placeholder="/path/to/repo"
                value={p.path}
                onChange={(e) => setProjects(updateAt(cfg.projects, i, { path: e.target.value }))}
              />
              <button className="btn btn--secondary" onClick={() => browseProject(i)}>Browse…</button>
              <select
                className="select"
                value={p.account}
                onChange={(e) => setProjects(updateAt(cfg.projects, i, { account: e.target.value }))}
              >
                {!cfg.accounts.some((a) => a.id === p.account) && (
                  <option value="">— account —</option>
                )}
                {cfg.accounts.map((a) => <option key={a.id} value={a.id}>{a.label}</option>)}
              </select>
              <button
                className="btn btn--icon"
                title="Remove project"
                onClick={() => setProjects(cfg.projects.filter((_, j) => j !== i))}
              >✕</button>
            </div>
            <textarea
              className="input project__roots"
              rows={2}
              placeholder="Worktree roots (optional) — e.g. ~/orca/workspaces/my-project, one per line or comma-separated"
              value={rootsDraft(p)}
              onChange={(e) => setRoots(i, e.target.value)}
            />
            {rootsErrors[i] && <p className="field-error">{rootsErrors[i]}</p>}
          </div>
        ))}
      </section>

      <section className="card">
        <div className="card__head">
          <h2 className="card__title">Inheritance</h2>
          {cfg.accounts.length > 0 && (
            <select
              className="select"
              value={inheritAccount}
              onChange={(e) => setInheritSel(e.target.value)}
            >
              {cfg.accounts.map((a) => <option key={a.id} value={a.id}>{a.label}</option>)}
            </select>
          )}
        </div>
        <p className="card__hint">Shared resources from <code>~/.claude</code> linked into each account. Toggle whether each subdir is merged in or skipped.</p>

        {cfg.accounts.length === 0 && <p className="muted">Add an account to see what it inherits.</p>}
        {inheritErr && <p className="banner banner--err">{inheritErr}</p>}
        {cfg.accounts.length > 0 && inheritRows.map((r) => {
          const mergeOn = r.decision === "merge" || (r.decision === null && r.status === "inherited");
          const skipOn = r.decision === "skip";
          return (
            <div className="row inherit-row" key={r.subdir}>
              <code className="inherit-row__name">{r.subdir}</code>
              <span className={`badge badge--${r.status}`}>{r.status}</span>
              {r.status !== "none" && (
                <span className="toggle">
                  <button
                    className={`toggle__btn ${mergeOn ? "toggle__btn--on" : ""}`}
                    onClick={() => onInheritToggle(r.subdir, "merge")}
                  >Merge</button>
                  <button
                    className={`toggle__btn ${skipOn ? "toggle__btn--on" : ""}`}
                    onClick={() => onInheritToggle(r.subdir, "skip")}
                  >Skip</button>
                </span>
              )}
            </div>
          );
        })}
        <p className="card__hint inherit-note"><code>plugins</code> — excluded by design.</p>
      </section>
    </>
  );
}
