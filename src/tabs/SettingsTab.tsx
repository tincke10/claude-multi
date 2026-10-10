import { Config, TerminalInfo } from "../api";

type Props = {
  config: Config;
  terminals: TerminalInfo[];
  patch: (next: Partial<Config>) => void;
};

export default function SettingsTab({ config, terminals, patch }: Props) {
  return (
    <>
    <section className="card">
      <h2 className="card__title">Terminal</h2>
      <p className="card__hint">Which terminal the tray opens for sessions and logins.</p>
      <select
        className="select"
        value={config.terminal}
        onChange={(e) => patch({ terminal: e.target.value })}
      >
        {terminals.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
      </select>
    </section>

    <section className="card">
      <div className="card__head">
        <h2 className="card__title">Usage</h2>
        <label className="check">
          <input
            type="checkbox"
            checked={config.usage.statusline_enabled}
            onChange={(e) =>
              patch({ usage: { ...config.usage, statusline_enabled: e.target.checked } })
            }
          />
          <span>Use the statusline to read real usage</span>
        </label>
      </div>
      <p className="card__hint">
        Claude Code reports your real 5-hour and 7-day limits to its statusline. When this is on,
        each account's <code>settings.json</code> runs <code>cms --statusline</code>, which records
        those numbers locally and then runs your own statusline, so what you see in Claude Code
        keeps showing exactly as before. Turning it off restores your original statusline.
      </p>
    </section>

    <section className="card">
      <div className="card__head">
        <h2 className="card__title">Account API (advanced)</h2>
        <label className="check">
          <input
            type="checkbox"
            checked={config.usage.endpoint_enabled}
            onChange={(e) =>
              patch({ usage: { ...config.usage, endpoint_enabled: e.target.checked } })
            }
          />
          <span>Read usage from Anthropic's account API (advanced)</span>
        </label>
      </div>
      <p className="card__hint">
        Adds exact per-model limits (for example Fable) and your plan. Off by default.
      </p>
      <p className="banner banner--warn" role="note">
        This API is undocumented, so it may break or start rate-limiting at any time. It sits in a
        grey area of Anthropic's terms: Claude Code's terms say developers may not collect, store
        or intermediate Claude.ai credentials or session tokens. Claude Multi only reads your own
        access token locally, in memory, and sends it to Anthropic's API alone. It never stores,
        refreshes or sends it anywhere else. The requests share rate limits with Claude Code, and
        macOS may ask for Keychain access.
      </p>
    </section>
    </>
  );
}
