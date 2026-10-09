import { describe, expect, it } from "vitest";
import {
  CLAUDE_DIR_PREFIX, canActOnAccount, duplicateDirIndices, nextId, normalizeSuffix, suffixOf, updateAt,
} from "./configDirs";

const acc = (id: string, config_dir: string) => ({ id, config_dir });

describe("suffixOf / normalizeSuffix", () => {
  it("test_should_strip_the_prefix_when_present", () => {
    expect(suffixOf(`${CLAUDE_DIR_PREFIX}work`)).toBe("work");
  });
  it("test_should_keep_dir_when_prefix_absent", () => {
    expect(suffixOf("/tmp/x")).toBe("/tmp/x");
  });
  it("test_should_ignore_case_and_trailing_separators_when_normalizing", () => {
    expect(normalizeSuffix("~/.claude-Work//")).toBe("work");
    expect(normalizeSuffix("~/.claude-Work\\")).toBe("work");
  });
});

describe("duplicateDirIndices", () => {
  it("test_should_return_empty_when_dirs_are_unique", () => {
    expect(duplicateDirIndices([acc("a", "~/.claude-a"), acc("b", "~/.claude-b")]).size).toBe(0);
  });
  it("test_should_flag_both_when_dirs_clash_ignoring_case", () => {
    const d = duplicateDirIndices([acc("a", "~/.claude-a"), acc("b", "~/.claude-B"), acc("c", "~/.claude-b/")]);
    expect([...d].sort()).toEqual([1, 2]);
  });
});

describe("nextId", () => {
  it("test_should_start_at_one_when_no_ids", () => {
    expect(nextId([], "a")).toBe("a1");
  });
  it("test_should_use_max_plus_one_ignoring_other_prefixes", () => {
    expect(nextId(["a1", "a7", "p9", "ax"], "a")).toBe("a8");
  });
});

describe("updateAt", () => {
  it("test_should_patch_only_the_given_index_immutably", () => {
    const src = [{ n: 1, m: 1 }, { n: 2, m: 2 }];
    const out = updateAt(src, 1, { n: 9 });
    expect(out).toEqual([{ n: 1, m: 1 }, { n: 9, m: 2 }]);
    expect(src[1].n).toBe(2);
  });
});

describe("canActOnAccount", () => {
  const snapshot = JSON.stringify({ accounts: [acc("a1", "~/.claude-a")] });
  it("test_should_allow_when_account_is_in_the_saved_snapshot", () => {
    expect(canActOnAccount(snapshot, acc("a1", "~/.claude-a"))).toBe(true);
  });
  it("test_should_deny_when_account_is_new", () => {
    expect(canActOnAccount(snapshot, acc("a2", "~/.claude-b"))).toBe(false);
  });
  it("test_should_deny_when_config_dir_was_edited", () => {
    expect(canActOnAccount(snapshot, acc("a1", "~/.claude-z"))).toBe(false);
  });
  it("test_should_deny_when_snapshot_is_unparseable", () => {
    expect(canActOnAccount("", acc("a1", "~/.claude-a"))).toBe(false);
  });
});
