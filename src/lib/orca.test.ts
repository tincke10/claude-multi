import { describe, expect, it } from "vitest";
import type { ImportCandidate, OrcaHealth, OrcaHealthCheck, Project } from "../api";
import {
  appendImported, isSelectable, levelSymbol, orcaPanelVisible, statusText, summarizeHealth, toSelections,
} from "./orca";

const check = (id: string, level: OrcaHealthCheck["level"]): OrcaHealthCheck =>
  ({ id, label: id, level, detail: "", hint: null });
const cand = (id: string, status: ImportCandidate["status"]): ImportCandidate =>
  ({ orca_repo_id: id, label: id, path: `/${id}`, worktree_roots: [], status });
const proj = (id: string): Project => ({ id, label: id, path: `/${id}`, account: "a1", worktree_roots: [] });

describe("orcaPanelVisible", () => {
  const health = (supported: boolean, level: OrcaHealthCheck["level"]): OrcaHealth =>
    ({ supported, checks: [check("orca_installed", level)] });
  it("test_should_show_only_when_supported_and_installed", () => {
    expect(orcaPanelVisible(null)).toBe(false);
    expect(orcaPanelVisible(health(false, "pass"))).toBe(false);
    expect(orcaPanelVisible(health(true, "fail"))).toBe(false);
    expect(orcaPanelVisible(health(true, "pass"))).toBe(true);
  });
});

describe("levelSymbol / summarizeHealth", () => {
  it("test_should_map_levels_to_symbols", () => {
    expect(levelSymbol("pass")).toBe("✓");
    expect(levelSymbol("warn")).toBe("!");
  });
  it("test_should_summarize_problems_and_warnings", () => {
    expect(summarizeHealth([check("a", "pass")])).toBe("All good");
    expect(summarizeHealth([check("a", "fail"), check("b", "warn"), check("c", "warn")]))
      .toBe("1 problem, 2 warnings");
  });
});

describe("import candidates", () => {
  const list = [
    cand("r1", { kind: "new" }),
    cand("r2", { kind: "already_mapped", project: "P" }),
    cand("r3", { kind: "missing" }),
    cand("r4", { kind: "new" }),
  ];
  it("test_should_only_select_new_candidates", () => {
    expect(list.map(isSelectable)).toEqual([true, false, false, true]);
  });
  it("test_should_describe_status", () => {
    expect(statusText(list[0])).toBe("New");
    expect(statusText(list[1])).toBe("Already mapped (P)");
    expect(statusText(list[2])).toBe("Folder missing");
  });
  it("test_should_build_selections_from_checked_selectable_rows_with_fallback_account", () => {
    const out = toSelections(list, new Set(["r1", "r2", "r4"]), { r1: "a2" }, "a1");
    expect(out).toEqual([
      { orca_repo_id: "r1", account_id: "a2" },
      { orca_repo_id: "r4", account_id: "a1" },
    ]);
  });
});

describe("appendImported", () => {
  it("test_should_append_without_replacing_local_projects", () => {
    const out = appendImported([proj("p1")], [proj("p2")]);
    expect(out.map((p) => p.id)).toEqual(["p1", "p2"]);
  });
  it("test_should_renumber_clashing_ids_after_the_local_max", () => {
    const out = appendImported([proj("p1"), proj("p3")], [proj("p1")]);
    expect(out.map((p) => p.id)).toEqual(["p1", "p3", "p4"]);
  });
  it("test_should_default_missing_worktree_roots", () => {
    const legacy = { ...proj("p2"), worktree_roots: undefined } as unknown as Project;
    expect(appendImported([], [legacy])[0].worktree_roots).toEqual([]);
  });
});
