import { useState } from "react";
import { Account, AccountOverview, AccountStatus, Config } from "../api";
import {
  AuthAction, ACTION_LABEL, actionsFor, isSaved, roomVariant,
} from "../lib/accounts";
import { vesselCaption } from "../lib/usageText";
import Chip from "../components/Chip";
import Room from "../components/Room";
import Vessel from "../components/Vessel";
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

const EMPTY_COPY = "This room is empty. Log in to start using and tracking this account.";

// Session and week from the current (local estimate) overview data. The
// per-model vessel appears only once real usage data carries models (Phase 4+).
function Vessels({ o }: { o: AccountOverview }) {
  return (
    <div className="vessels">
      <Vessel name="Session" hint="5 hours" percent={o.session.percent} caption={vesselCaption(o.session)} />
      <Vessel name="Week" hint="7 days" percent={o.week.percent} caption={vesselCaption(o.week)} />
    </div>
  );
}

function RoomActions(props: {
  status: AccountStatus;
  busy: boolean;
  onAction: (a: AuthAction) => void;
}) {
  const { status, busy, onAction } = props;
  return (
    <div className="room__acts">
      {actionsFor(status).map((act, idx) => (
        <button
          key={act}
          className={`btn ${idx === 0 && status !== "logged_in" ? "btn--primary" : ""}`}
          disabled={busy}
          onClick={() => onAction(act)}
        >{ACTION_LABEL[act]}</button>
      ))}
    </div>
  );
}

export default function AccountsTab({ config: cfg, snapshot, patch, accountsOverview: ov }: Props) {
  // Accounts sharing a config dir would break isolation: flag them, block Save.
  const dupDirs = duplicateDirIndices(cfg.accounts);
  const setAccounts = (accounts: Account[]) => patch({ accounts });
  // A freshly added account opens its edit section so it can be named right away.
  const [openId, setOpenId] = useState<string | null>(null);

  const addAccount = () => {
    const id = nextId(cfg.accounts.map((a) => a.id), "a");
    setAccounts([...cfg.accounts, {
      id, label: "New account", config_dir: `${CLAUDE_DIR_PREFIX}new`,
      inherit_overrides: {},
      usage_limits: { session_tokens: null, weekly_tokens: null },
    }]);
    setOpenId(id);
  };

  return (
    <section aria-label="Accounts">
      <div className="rooms__bar">
        <h2 className="rooms__title">Accounts</h2>
        <div className="card__actions">
          <button
            className="btn btn--icon btn--refresh"
            title="Refresh login status and usage"
            aria-label="Refresh login status and usage"
            onClick={() => void ov.refresh()}
            disabled={ov.refreshing}
          >↻</button>
          <button className="btn btn--ghost" onClick={addAccount}>Add account</button>
        </div>
      </div>

      {ov.error && <p className="field-error">Couldn't read account status: {ov.error}</p>}
      {cfg.accounts.length === 0 && (
        <p className="muted">No accounts yet. Add one to give it its own sealed config directory.</p>
      )}
      <div className="rooms">
        {cfg.accounts.map((a, i) => {
          const row = ov.overview.find((o) => o.id === a.id);
          const saved = isSaved(row, a.id, a.config_dir) && canActOnAccount(snapshot, a);
          const status = saved && row ? row.status : null;
          const err = ov.actionErrs[a.id];
          return (
            <Room
              key={a.id}
              variant={status === null ? "empty" : roomVariant(status)}
              name={a.label.trim() || a.id}
              email={status === "logged_out" ? null : row?.email}
              dir={a.config_dir}
            >
              {status === null && (
                <p className="room__empty">
                  {ov.loaded ? "Save first to log in and see usage." : "Checking…"}
                </p>
              )}
              {row && status === "logged_out" && <p className="room__empty">{EMPTY_COPY}</p>}
              {row && status !== null && status !== "logged_out" && (
                <>
                  <Vessels o={row} />
                  {status === "expired" && (
                    <div className="room__chips"><Chip kind="warn">Session expired</Chip></div>
                  )}
                </>
              )}
              {err && <p className="field-error">{err}</p>}
              {row && status !== null && (
                <RoomActions
                  status={status}
                  busy={ov.busy.has(a.id)}
                  onAction={(act) => void ov.runAuthAction(a.id, act)}
                />
              )}
              <details className="room__edit" open={openId === a.id ? true : undefined}>
                <summary>Edit</summary>
                <div className="row">
                  <input
                    className="input row__grow"
                    aria-label="Account label"
                    placeholder="Label (e.g. Personal)"
                    value={a.label}
                    onChange={(e) => setAccounts(updateAt(cfg.accounts, i, { label: e.target.value }))}
                  />
                  <button
                    className="btn btn--icon"
                    title="Remove account"
                    aria-label={`Remove ${a.label || a.id}`}
                    onClick={() => setAccounts(cfg.accounts.filter((_, j) => j !== i))}
                  >✕</button>
                </div>
                <span className={`affix ${dupDirs.has(i) ? "affix--invalid" : ""}`}>
                  <span className="affix__prefix">{CLAUDE_DIR_PREFIX}</span>
                  <input
                    className="input affix__input"
                    aria-label="Config directory suffix"
                    placeholder="suffix"
                    value={suffixOf(a.config_dir)}
                    onChange={(e) =>
                      setAccounts(updateAt(cfg.accounts, i, { config_dir: CLAUDE_DIR_PREFIX + e.target.value }))
                    }
                  />
                </span>
                {dupDirs.has(i) && (
                  <p className="field-error">
                    Another account already uses <code>{a.config_dir}</code>. Each account needs its own config directory.
                  </p>
                )}
              </details>
            </Room>
          );
        })}
      </div>
    </section>
  );
}
