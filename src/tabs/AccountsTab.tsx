import { Account, AccountOverview, Config, UsageLimits, UsageView } from "../api";
import {
  AuthAction, ACTION_LABEL, actionsFor, isSaved, statusPill, usageText,
} from "../lib/accounts";
import {
  CLAUDE_DIR_PREFIX, canActOnAccount, duplicateDirIndices, nextId, suffixOf, updateAt,
} from "../lib/configDirs";

type OverviewState = {
  overview: AccountOverview[];
  loaded: boolean;
  error: string | null;
  refreshing: boolean;
  actionErrs: Record<string, string>;
  busy: Set<string>;
  refresh: () => Promise<unknown>;
  runAuthAction: (id: string, action: AuthAction) => Promise<void>;
};

type Props = {
  config: Config;
  snapshot: string;
  patch: (next: Partial<Config>) => void;
  accountsOverview: OverviewState;
};

function UsageBar({ name, u }: { name: string; u: UsageView }) {
  return (
    <div className="usage">
      <span className="usage__name">{name}</span>
      <span className="usage__track" aria-hidden="true">
        {u.percent !== null && (
          <span
            className={`usage__fill ${u.percent >= 90 ? "usage__fill--hot" : ""}`}
            style={{ width: `${u.percent}%` }}
          />
        )}
      </span>
      <span className="usage__text">{usageText(u)}</span>
    </div>
  );
}

function AccountStatusRow(props: {
  overview: AccountOverview | undefined;
  saved: boolean;
  loaded: boolean;
  busy: boolean;
  error: string | undefined;
  onAction: (a: AuthAction) => void;
}) {
  const { overview: o, saved, loaded, busy, error, onAction } = props;
  if (!saved || !o) {
    return (
      <div className="row account__status">
        <span className="muted">{loaded ? "Save first to log in and see usage." : "Checking…"}</span>
      </div>
    );
  }
  const pill = statusPill(o);
  return (
    <>
      <div className="row account__status">
        <span className={`pill pill--${pill.tone}`}>{pill.text}</span>
        <span className="row__grow" />
        {actionsFor(o.status).map((act) => (
          <button
            key={act}
            className="btn btn--secondary btn--compact"
            disabled={busy}
            onClick={() => onAction(act)}
          >{ACTION_LABEL[act]}</button>
        ))}
      </div>
      {error && <p className="field-error">{error}</p>}
      <UsageBar name="Session (5h)" u={o.session} />
      <UsageBar name="Week (7d)" u={o.week} />
    </>
  );
}

// Parse a token-ceiling input: empty => null (unset); otherwise a non-negative
// integer, ignoring anything that isn't a clean number.
const parseCeiling = (raw: string): number | null => {
  const t = raw.trim();
  if (t === "") return null;
  const n = Number(t);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : null;
};

export default function AccountsTab({ config: cfg, snapshot, patch, accountsOverview: ov }: Props) {
  // Accounts sharing a config dir would break isolation: flag them, block Save.
  const dupDirs = duplicateDirIndices(cfg.accounts);
  const setAccounts = (accounts: Account[]) => patch({ accounts });
  const setAccountLimits = (i: number, u: Partial<UsageLimits>) =>
    setAccounts(updateAt(cfg.accounts, i, { usage_limits: { ...cfg.accounts[i].usage_limits, ...u } }));

  const addAccount = () => {
    const id = nextId(cfg.accounts.map((a) => a.id), "a");
    setAccounts([...cfg.accounts, {
      id, label: "New account", config_dir: `${CLAUDE_DIR_PREFIX}new`,
      inherit_overrides: {},
      usage_limits: { session_tokens: null, weekly_tokens: null },
    }]);
  };

  return (
    <section className="card">
      <div className="card__head">
        <h2 className="card__title">Accounts</h2>
        <div className="card__actions">
          <button
            className="btn btn--icon btn--refresh"
            title="Refresh login status and usage"
            aria-label="Refresh login status and usage"
            onClick={() => void ov.refresh()}
            disabled={ov.refreshing}
          >↻</button>
          <button className="btn btn--ghost" onClick={addAccount}>+ Add account</button>
        </div>
      </div>
      <p className="card__hint">
        Each account is isolated in its own <code>{CLAUDE_DIR_PREFIX}…</code> config directory.
        The tray shows its rolling token usage — <code>Session (5h)</code> and{" "}
        <code>Week (7d)</code>. Set an approximate per-account token ceiling to see{" "}
        <code>used / ceiling · %</code> (plans differ, so a work account may allow more than a
        personal one). There's no local source for Anthropic's real limits, so calibrate: when
        {" "}<code>/usage</code> shows e.g. 13% used, the ceiling ≈ current tokens ÷ 0.13. Leave
        blank to show raw tokens.
      </p>

      {ov.error && <p className="field-error">Couldn't read account status: {ov.error}</p>}
      {cfg.accounts.length === 0 && <p className="muted">No accounts yet.</p>}
      {cfg.accounts.map((a, i) => {
        const row = ov.overview.find((o) => o.id === a.id);
        return (
          <div className="account" key={a.id}>
            <div className="row">
              <input
                className="input row__grow"
                placeholder="Label (e.g. Personal)"
                value={a.label}
                onChange={(e) => setAccounts(updateAt(cfg.accounts, i, { label: e.target.value }))}
              />
              <span className={`affix ${dupDirs.has(i) ? "affix--invalid" : ""}`}>
                <span className="affix__prefix">{CLAUDE_DIR_PREFIX}</span>
                <input
                  className="input affix__input"
                  placeholder="suffix"
                  value={suffixOf(a.config_dir)}
                  onChange={(e) =>
                    setAccounts(updateAt(cfg.accounts, i, { config_dir: CLAUDE_DIR_PREFIX + e.target.value }))
                  }
                />
              </span>
              <button
                className="btn btn--icon"
                title="Remove account"
                onClick={() => setAccounts(cfg.accounts.filter((_, j) => j !== i))}
              >✕</button>
            </div>
            {dupDirs.has(i) && (
              <p className="field-error">
                Another account already uses <code>{a.config_dir}</code>. Each account needs its own config directory.
              </p>
            )}
            <AccountStatusRow
              overview={row}
              saved={isSaved(row, a.id, a.config_dir) && canActOnAccount(snapshot, a)}
              loaded={ov.loaded}
              busy={ov.busy.has(a.id)}
              error={ov.actionErrs[a.id]}
              onAction={(act) => void ov.runAuthAction(a.id, act)}
            />
            <div className="row account__limits">
              <label className="field">
                <span className="field__label">Session (5h) ceiling</span>
                <input
                  className="input"
                  type="number"
                  min={0}
                  placeholder="tokens, e.g. 5000000"
                  value={a.usage_limits.session_tokens ?? ""}
                  onChange={(e) => setAccountLimits(i, { session_tokens: parseCeiling(e.target.value) })}
                />
              </label>
              <label className="field">
                <span className="field__label">Week (7d) ceiling</span>
                <input
                  className="input"
                  type="number"
                  min={0}
                  placeholder="tokens, e.g. 40000000"
                  value={a.usage_limits.weekly_tokens ?? ""}
                  onChange={(e) => setAccountLimits(i, { weekly_tokens: parseCeiling(e.target.value) })}
                />
              </label>
            </div>
          </div>
        );
      })}
    </section>
  );
}
