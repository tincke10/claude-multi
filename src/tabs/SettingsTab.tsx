import { Config, TerminalInfo } from "../api";

type Props = {
  config: Config;
  terminals: TerminalInfo[];
  patch: (next: Partial<Config>) => void;
};

export default function SettingsTab({ config, terminals, patch }: Props) {
  return (
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
  );
}
