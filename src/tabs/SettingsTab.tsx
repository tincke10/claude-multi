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
    </>
  );
}
