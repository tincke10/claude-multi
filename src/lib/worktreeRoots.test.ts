import { describe, expect, it } from "vitest";
import { formatWorktreeRoots, parseWorktreeRoots, worktreeRootsProblem } from "./worktreeRoots";

describe("parseWorktreeRoots", () => {
  it("test_should_split_on_commas_and_newlines_and_drop_empties", () => {
    expect(parseWorktreeRoots(" ~/a, ~/b\n\n~/c,\r\n,")).toEqual(["~/a", "~/b", "~/c"]);
  });
  it("test_should_return_empty_for_blank_input", () => {
    expect(parseWorktreeRoots("  ")).toEqual([]);
  });
});

describe("formatWorktreeRoots", () => {
  it("test_should_join_with_newlines", () => {
    expect(formatWorktreeRoots(["~/a", "~/b"])).toBe("~/a\n~/b");
  });
});

describe("worktreeRootsProblem", () => {
  it("test_should_accept_distinct_roots", () => {
    expect(worktreeRootsProblem(["~/a", "~/b"])).toBeNull();
  });
  it("test_should_flag_duplicates_ignoring_case_and_trailing_separator", () => {
    expect(worktreeRootsProblem(["~/a/", "~/A"])).toBe("Duplicate worktree root: ~/A");
  });
  it("test_should_flag_empty_entries", () => {
    expect(worktreeRootsProblem(["~/a", " "])).toBe("Worktree roots can't contain empty entries.");
  });
});
