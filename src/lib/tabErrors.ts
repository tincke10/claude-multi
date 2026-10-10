import type { Config } from "../api";
import { duplicateDirIndices } from "./configDirs";
import { TAB_IDS, type TabId } from "./lastTab";
import { trackingProblem } from "./tracking";
import { worktreeRootsProblem } from "./worktreeRoots";

export type TabErrors = Record<TabId, string | null>;

// One message (or null) per tab; drives the error dots and disables Save.
export function tabErrors(cfg: Config): TabErrors {
  const rootsErr = cfg.projects.map((p) => worktreeRootsProblem(p.worktree_roots)).find((e) => e !== null) ?? null;
  return {
    accounts: duplicateDirIndices(cfg.accounts).size > 0
      ? "Fix the duplicate account config directories to save."
      : null,
    projects: rootsErr,
    tracking: trackingProblem(cfg.tracking),
    orca: null,
    settings: null,
  };
}

// First tab (in tab order) that has an error, for the footer hint.
export const firstErrorTab = (errors: TabErrors): TabId | null =>
  TAB_IDS.find((t) => errors[t] !== null) ?? null;
