import { describe, expect, it } from "vitest";
import type { Config } from "../api";
import { firstErrorTab, tabErrors } from "./tabErrors";

const base = (): Config => ({
  terminal: "iterm",
  accounts: [
    { id: "a1", label: "A", config_dir: "~/.claude-a", inherit_overrides: {}, usage_limits: { session_tokens: null, weekly_tokens: null } },
    { id: "a2", label: "B", config_dir: "~/.claude-b", inherit_overrides: {}, usage_limits: { session_tokens: null, weekly_tokens: null } },
  ],
  projects: [{ id: "p1", label: "x", path: "/x", account: "a1", worktree_roots: [] }],
  tracking: {
    enabled: false, ingest_url: "", ingest_token: "", grafana_otlp_endpoint: "",
    grafana_instance_id: "", grafana_token: "", summaries: false,
  },
});

describe("tabErrors", () => {
  it("test_should_have_no_errors_when_config_is_valid", () => {
    const e = tabErrors(base());
    expect(Object.values(e).every((v) => v === null)).toBe(true);
    expect(firstErrorTab(e)).toBeNull();
  });
  it("test_should_flag_accounts_when_two_share_a_config_dir", () => {
    const c = base();
    c.accounts[1].config_dir = "~/.claude-A/";
    const e = tabErrors(c);
    expect(e.accounts).not.toBeNull();
    expect(firstErrorTab(e)).toBe("accounts");
  });
  it("test_should_flag_projects_when_worktree_root_is_duplicated", () => {
    const c = base();
    c.projects[0].worktree_roots = ["~/a", "~/A/"];
    const e = tabErrors(c);
    expect(e.projects).toMatch(/Duplicate worktree root/);
    expect(e.accounts).toBeNull();
  });
  it("test_should_flag_tracking_when_enabled_without_url", () => {
    const c = base();
    c.tracking.enabled = true;
    expect(tabErrors(c).tracking).toBe("Tracking needs the ingest URL.");
  });
  it("test_should_report_first_error_tab_in_tab_order", () => {
    const c = base();
    c.tracking.enabled = true;
    c.projects[0].worktree_roots = ["~/a", "~/a"];
    expect(firstErrorTab(tabErrors(c))).toBe("projects");
  });
  it("test_should_never_flag_orca_or_settings", () => {
    const e = tabErrors(base());
    expect(e.orca).toBeNull();
    expect(e.settings).toBeNull();
  });
});
