import { useEffect, useState } from "react";
import {
  Account, ImportList, OrcaHealth, Project,
  orcaHealth, listOrcaImportCandidates, importOrcaRepos,
} from "./api";
import {
  isSelectable, levelSymbol, orcaPanelVisible, statusText, summarizeHealth, toSelections,
} from "./orca";

type Props = {
  accounts: Account[];
  // Receives the projects the backend already persisted; the parent appends
  // them to its local state without reloading (so unsaved edits survive).
  onImported: (added: Project[]) => void;
};

// Orca integration card: read-only health of how Orca is wired to cms, plus
// importing Orca repos as projects. Hidden unless the backend supports it and
// Orca is installed.
export default function OrcaPanel({ accounts, onImported }: Props) {
  const [health, setHealth] = useState<OrcaHealth | null>(null);
  const [checking, setChecking] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  // Import dialog
  const [dialogOpen, setDialogOpen] = useState(false);
  const [list, setList] = useState<ImportList | null>(null);
  const [loadingList, setLoadingList] = useState(false);
  const [importErr, setImportErr] = useState<string | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [accountFor, setAccountFor] = useState<Record<string, string>>({});
  const [importing, setImporting] = useState(false);

  const runCheck = async () => {
    setChecking(true);
    setErr(null);
    try {
      setHealth(await orcaHealth());
    } catch (e) {
      setErr(`${e}`);
    } finally {
      setChecking(false);
    }
  };

  useEffect(() => { void runCheck(); }, []);

  const openDialog = async () => {
    setDialogOpen(true);
    setList(null);
    setImportErr(null);
    setChecked(new Set());
    setAccountFor({});
    setLoadingList(true);
    try {
      setList(await listOrcaImportCandidates());
    } catch (e) {
      setImportErr(`${e}`);
    } finally {
      setLoadingList(false);
    }
  };

  const fallback = accounts[0]?.id ?? "";
  const selections = list ? toSelections(list.candidates, checked, accountFor, fallback) : [];

  const doImport = async () => {
    setImporting(true);
    setImportErr(null);
    try {
      const added = await importOrcaRepos(selections);
      onImported(added);
      setDialogOpen(false);
      void runCheck(); // mapping changed: refresh the "unmapped repos" row
    } catch (e) {
      setImportErr(`${e}`);
    } finally {
      setImporting(false);
    }
  };

  const toggle = (id: string) => {
    const next = new Set(checked);
    if (next.has(id)) next.delete(id); else next.add(id);
    setChecked(next);
  };

  if (!orcaPanelVisible(health)) return null;
  const checks = health?.checks ?? [];

  return (
    <section className="card">
      <div className="card__head">
        <h2 className="card__title">Orca</h2>
        <div className="row">
          <button className="btn btn--secondary" onClick={openDialog} disabled={accounts.length === 0}>
            Import from Orca…
          </button>
          <button className="btn btn--secondary" onClick={runCheck} disabled={checking}>
            {checking ? "Checking…" : "Check"}
          </button>
        </div>
      </div>
      <p className="card__hint">
        How Orca is wired to <code>cms</code>. Read-only: nothing here changes Orca&apos;s settings.
        {" "}<strong>{summarizeHealth(checks)}</strong>
      </p>
      {err && <p className="field-error">Check failed: {err}</p>}
      <ul className="orca-checks">
        {checks.map((c) => (
          <li key={c.id} className={`orca-check orca-check--${c.level}`}>
            <span className="orca-check__icon" aria-label={c.level}>{levelSymbol(c.level)}</span>
            <div className="orca-check__body">
              <div className="orca-check__label">{c.label}</div>
              <div className="orca-check__detail">{c.detail}</div>
              {c.hint && <div className="orca-check__hint">{c.hint}</div>}
            </div>
          </li>
        ))}
      </ul>

      {dialogOpen && (
        <div className="modal" role="dialog" aria-modal="true" aria-label="Import from Orca">
          <div className="modal__panel">
            <div className="card__head">
              <h2 className="card__title">Import from Orca</h2>
              <button className="btn btn--icon" title="Close" onClick={() => setDialogOpen(false)}>✕</button>
            </div>
            <p className="card__hint">
              Pick Orca repos to add as projects. Worktree roots are derived from the worktrees Orca
              created. Nothing is written to Orca, and the projects are saved right away.
            </p>
            {loadingList && <p className="muted">Reading Orca…</p>}
            {list?.truncated && (
              <p className="banner banner--warn">
                Orca&apos;s worktree list was cut off, so some worktree roots may be missing.
              </p>
            )}
            {list && list.candidates.length === 0 && <p className="muted">Orca has no repos.</p>}
            {list && list.candidates.map((c) => {
              const ok = isSelectable(c);
              return (
                <div className={`import-row ${ok ? "" : "import-row--off"}`} key={c.orca_repo_id}>
                  <input
                    type="checkbox"
                    disabled={!ok}
                    checked={ok && checked.has(c.orca_repo_id)}
                    onChange={() => toggle(c.orca_repo_id)}
                  />
                  <div className="import-row__body">
                    <div className="import-row__label">{c.label}</div>
                    <div className="import-row__path">{c.path}</div>
                    {c.worktree_roots.length > 0 && (
                      <div className="import-row__path">roots: {c.worktree_roots.join(", ")}</div>
                    )}
                  </div>
                  <span className={`badge badge--${ok ? "inherited" : "none"}`}>{statusText(c)}</span>
                  <select
                    className="select"
                    disabled={!ok}
                    value={accountFor[c.orca_repo_id] ?? fallback}
                    onChange={(e) => setAccountFor({ ...accountFor, [c.orca_repo_id]: e.target.value })}
                  >
                    {accounts.map((a) => <option key={a.id} value={a.id}>{a.label}</option>)}
                  </select>
                </div>
              );
            })}
            {importErr && <p className="field-error">{importErr}</p>}
            <div className="row modal__footer">
              <button
                className="btn btn--primary"
                onClick={doImport}
                disabled={selections.length === 0 || importing}
              >
                {importing ? "Importing…" : `Import ${selections.length || ""}`.trim()}
              </button>
              <button className="btn btn--secondary" onClick={() => setDialogOpen(false)}>Cancel</button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
