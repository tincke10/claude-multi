import { useCallback, useEffect, useRef, useState } from "react";
import { Config, TerminalInfo, getConfig, listTerminals, saveConfig } from "../api";
import { isDirty, snapshotOf } from "../lib/dirty";

// `invoke` only works inside the Tauri webview; in a plain browser the IPC
// bridge is absent.
export const IN_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Status = { kind: "ok" | "err"; text: string };

// Heal legacy / inconsistent configs so every input binds.
function normalize(cfg: Config): Config {
  // Projects whose account is empty or points at a removed account (e.g.
  // migrated from the old default_account field) go to the first account so
  // they aren't silently invisible in the tray.
  const validIds = new Set(cfg.accounts.map((a) => a.id));
  const firstId = cfg.accounts[0]?.id ?? "";
  const projects = cfg.projects.map((p) => ({
    ...p,
    // Legacy projects may predate worktree_roots.
    worktree_roots: p.worktree_roots ?? [],
    account: validIds.has(p.account) ? p.account : firstId,
  }));
  // Legacy accounts may predate usage_limits / inherit_overrides.
  const accounts = cfg.accounts.map((a) => ({
    ...a,
    usage_limits: a.usage_limits ?? { session_tokens: null, weekly_tokens: null },
    inherit_overrides: a.inherit_overrides ?? {},
  }));
  return { ...cfg, accounts, projects };
}

// Owns the editable config, the last-saved snapshot (dirty = differs from it)
// and the global Save. `onSaved` lets the shell refresh dependent views.
export function useConfig(onSaved: () => void) {
  const [config, setConfig] = useState<Config | null>(null);
  const [snapshot, setSnapshot] = useState("");
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const [loadError, setLoadError] = useState<string | null>(IN_TAURI
    ? null
    : "Open this window from the claude-multi tray icon (run `npm run tauri dev`). It can't run in a plain browser.");
  const [status, setStatus] = useState<Status | null>(null);
  const [saveWarnings, setSaveWarnings] = useState<string[]>([]);
  const onSavedRef = useRef(onSaved);
  onSavedRef.current = onSaved;

  useEffect(() => {
    if (!IN_TAURI) return;
    (async () => {
      try {
        const [cfg, terms] = await Promise.all([getConfig(), listTerminals()]);
        const healed = normalize(cfg);
        setConfig(healed);
        setSnapshot(snapshotOf(healed));
        setTerminals(terms);
      } catch (e) {
        setLoadError(`Failed to load config: ${e}`);
      }
    })();
  }, []);

  const patch = useCallback(
    (next: Partial<Config>) => setConfig((c) => c && { ...c, ...next }),
    [],
  );

  // For edits the backend already persisted (inherit toggle, Orca import):
  // apply to both the form and the snapshot so they don't mark it dirty.
  const applyPersisted = useCallback((fn: (c: Config) => Config) => {
    setConfig((c) => c && fn(c));
    setSnapshot((s) => {
      try { return snapshotOf(fn(JSON.parse(s) as Config)); } catch { return s; }
    });
  }, []);

  const clearStatus = useCallback(() => { setStatus(null); setSaveWarnings([]); }, []);

  const save = async () => {
    if (!config) return;
    const saved = config;
    try {
      const warnings = await saveConfig(saved);
      setSaveWarnings(warnings);
      onSavedRef.current();
      // Re-fetch tracking so freshly typed tokens go back to the mask: the
      // plain value must not linger in UI state after it is stored.
      const fresh = await getConfig();
      setConfig((c) => c && { ...c, tracking: fresh.tracking });
      setSnapshot(snapshotOf({ ...saved, tracking: fresh.tracking }));
      setStatus({
        kind: "ok",
        text: warnings.length > 0
          ? `Saved with ${warnings.length} warning(s) — see Tracking.`
          : "Saved — the tray menu was updated.",
      });
      window.setTimeout(() => setStatus(null), 3000);
    } catch (e) {
      setStatus({ kind: "err", text: `Save failed: ${e}` });
    }
  };

  return {
    config, snapshot, terminals, loadError, status, saveWarnings,
    dirty: isDirty(config, snapshot),
    patch, applyPersisted, save, clearStatus,
  };
}
