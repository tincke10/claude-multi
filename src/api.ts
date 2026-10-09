import { invoke } from "@tauri-apps/api/core";

// Calibrated token ceilings for an account's tray usage lines. `null` = no
// ceiling set (the tray then shows raw tokens instead of a percentage).
export type UsageLimits = { session_tokens: number | null; weekly_tokens: number | null };
export type InheritDecision = "merge" | "skip";
// Owned by the backend (tray prompt / set_inherit_decision); the UI only mirrors
// it locally and save_config ignores the incoming value in favor of the disk's.
export type Account = {
  id: string;
  label: string;
  config_dir: string;
  inherit_overrides: Partial<Record<string, InheritDecision>>;
  usage_limits: UsageLimits;
};
export type Project = {
  id: string; label: string; path: string; account: string;
  // Extra folders (e.g. Orca worktree parents) mapped to this project by `cms --auto`.
  worktree_roots: string[];
};
// Opt-in session tracking, edited by the Preferences Tracking section. The
// backend never sends the real tokens to the UI: a saved token arrives as
// TOKEN_MASK, and sending TOKEN_MASK back on save means "keep the stored one".
// Must match `tracking::TOKEN_MASK` in the backend.
export const TOKEN_MASK = "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022";
export type TrackingConfig = {
  enabled: boolean;
  ingest_url: string;
  ingest_token: string;
  grafana_otlp_endpoint: string;
  grafana_instance_id: string;
  grafana_token: string;
  summaries: boolean;
};
// Result of "Test connection". Tokens are never echoed back.
export type TrackingCheck = {
  ingest_ok: boolean;
  ingest_message: string;
  grafana_ok: boolean | null;
  grafana_message: string | null;
  cms_path: string | null;
  warnings: string[];
};
// Where real usage numbers come from. Mirrors `config::UsageSourcesConfig`.
export type UsageSourcesConfig = {
  // Install `cms --statusline` per account to read rate limits from Claude Code's statusline.
  statusline_enabled: boolean;
  // Opt-in (later phase): Anthropic's undocumented OAuth usage endpoint.
  endpoint_enabled: boolean;
};
export type Config = {
  terminal: string;
  accounts: Account[];
  projects: Project[];
  tracking: TrackingConfig;
  usage: UsageSourcesConfig;
};
export type TerminalInfo = { id: string; label: string };

export type InheritStatus = "inherited" | "skipped" | "conflict" | "none";
export type InheritSubdirStatus = {
  subdir: string;
  status: InheritStatus;
  decision: InheritDecision | null;
};

// Rejects with the backend error string (e.g. corrupt config.json + backup path).
export const getConfig = () => invoke<Config>("get_config");
// Resolves to non-fatal warnings (e.g. tracking hooks that could not be installed).
export const saveConfig = (config: Config) => invoke<string[]>("save_config", { config });
export const listTerminals = () => invoke<TerminalInfo[]>("list_terminals");

export const getInheritStatus = (accountId: string) =>
  invoke<InheritSubdirStatus[]>("get_inherit_status", { accountId });
export const setInheritDecision = (
  accountId: string,
  subdir: string,
  decision: InheritDecision,
) => invoke<void>("set_inherit_decision", { accountId, subdir, decision });

export const testTrackingConnection = (tracking: TrackingConfig) =>
  invoke<TrackingCheck>("test_tracking_connection", { tracking });

// Orca integration (macOS only). Everything Orca-related is read-only.
export type HealthLevel = "pass" | "warn" | "fail" | "info" | "unknown";
export type OrcaHealthCheck = {
  id: string;
  label: string;
  level: HealthLevel;
  detail: string;
  hint: string | null;
};
// `supported` is false off macOS, where the Orca card is not shown at all.
export type OrcaHealth = { supported: boolean; checks: OrcaHealthCheck[] };
export const orcaHealth = () => invoke<OrcaHealth>("orca_health");

// Import from Orca. Candidate/selection fields are snake_case like Project.
export type CandidateStatus =
  | { kind: "new" }
  | { kind: "already_mapped"; project: string }
  | { kind: "missing" };
export type ImportCandidate = {
  orca_repo_id: string;
  label: string;
  path: string;
  worktree_roots: string[];
  status: CandidateStatus;
};
export type ImportList = {
  source: "cli" | "sqlite";
  // The worktree list was cut off, so derived worktree roots may be incomplete.
  truncated: boolean;
  candidates: ImportCandidate[];
};
export type ImportSelection = { orca_repo_id: string; account_id: string };
export const listOrcaImportCandidates = () => invoke<ImportList>("list_orca_import_candidates");
// Resolves to the projects the backend persisted (it never reloads the window's state).
export const importOrcaRepos = (selections: ImportSelection[]) =>
  invoke<Project[]>("import_orca_repos", { selections });

// Preferences Accounts card: login status + rolling usage per saved account.
export type AccountStatus = "logged_in" | "logged_out" | "expired";
// `percent` is clamped to 100 and null without a ceiling (show raw tokens).
export type UsageView = { used: number; ceiling: number | null; percent: number | null };
export type AccountOverview = {
  id: string;
  label: string;
  config_dir: string;
  status: AccountStatus;
  email: string | null;
  dir_exists: boolean;
  session: UsageView;
  week: UsageView;
};
export const accountsOverview = () => invoke<AccountOverview[]>("accounts_overview");
// These open the configured terminal; the outcome shows up later in the overview.
export const loginAccount = (accountId: string) => invoke<void>("login_account", { accountId });
export const logoutAccount = (accountId: string) => invoke<void>("logout_account", { accountId });
export const reloginAccount = (accountId: string) => invoke<void>("relogin_account", { accountId });
