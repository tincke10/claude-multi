import { describe, expect, it } from "vitest";
import { TAB_IDS, loadTab, saveTab } from "./lastTab";

const memory = (initial: Record<string, string> = {}) => {
  const data = { ...initial };
  return {
    data,
    getItem: (k: string) => data[k] ?? null,
    setItem: (k: string, v: string) => { data[k] = v; },
  };
};
const throwing = {
  getItem: () => { throw new Error("denied"); },
  setItem: () => { throw new Error("denied"); },
};

describe("TAB_IDS", () => {
  it("test_should_list_the_five_tabs_in_order", () => {
    expect(TAB_IDS).toEqual(["accounts", "projects", "tracking", "orca", "settings"]);
  });
});

describe("loadTab", () => {
  it("test_should_return_stored_tab_when_valid", () => {
    expect(loadTab(memory({ "cm.prefs.tab": "tracking" }))).toBe("tracking");
  });
  it("test_should_default_to_accounts_when_nothing_stored", () => {
    expect(loadTab(memory())).toBe("accounts");
  });
  it("test_should_default_to_accounts_when_value_unknown", () => {
    expect(loadTab(memory({ "cm.prefs.tab": "nope" }))).toBe("accounts");
  });
  it("test_should_default_to_accounts_when_storage_throws", () => {
    expect(loadTab(throwing)).toBe("accounts");
  });
  it("test_should_default_to_accounts_when_storage_missing", () => {
    expect(loadTab(null)).toBe("accounts");
  });
});

describe("saveTab", () => {
  it("test_should_persist_tab_under_the_key", () => {
    const s = memory();
    saveTab("settings", s);
    expect(s.data["cm.prefs.tab"]).toBe("settings");
  });
  it("test_should_not_throw_when_storage_throws", () => {
    expect(() => saveTab("settings", throwing)).not.toThrow();
  });
  it("test_should_not_throw_when_storage_missing", () => {
    expect(() => saveTab("settings", null)).not.toThrow();
  });
});
