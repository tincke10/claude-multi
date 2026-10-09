import { useEffect, useState } from "react";
import { OrcaHealth, orcaHealth } from "./api";
import { levelSymbol, orcaPanelVisible, summarizeHealth } from "./orca";

// Orca integration card: read-only health of how Orca is wired to cms.
// Hidden unless the backend supports it and Orca is installed.
export default function OrcaPanel() {
  const [health, setHealth] = useState<OrcaHealth | null>(null);
  const [checking, setChecking] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const runCheck = async () => {
    setChecking(true);
    setErr(null);
    try {
      setHealth(await orcaHealth());
    } catch (e) {
      setErr(`${e}`);
    } finally {
      setChecking(false);
    }
  };

  useEffect(() => { void runCheck(); }, []);

  if (!orcaPanelVisible(health)) return null;
  const checks = health?.checks ?? [];

  return (
    <section className="card">
      <div className="card__head">
        <h2 className="card__title">Orca</h2>
        <button className="btn btn--secondary" onClick={runCheck} disabled={checking}>
          {checking ? "Checking…" : "Check"}
        </button>
      </div>
      <p className="card__hint">
        How Orca is wired to <code>cms</code>. Read-only: nothing here changes Orca&apos;s settings.
        {" "}<strong>{summarizeHealth(checks)}</strong>
      </p>
      {err && <p className="field-error">Check failed: {err}</p>}
      <ul className="orca-checks">
        {checks.map((c) => (
          <li key={c.id} className={`orca-check orca-check--${c.level}`}>
            <span className="orca-check__icon" aria-label={c.level}>{levelSymbol(c.level)}</span>
            <div className="orca-check__body">
              <div className="orca-check__label">{c.label}</div>
              <div className="orca-check__detail">{c.detail}</div>
              {c.hint && <div className="orca-check__hint">{c.hint}</div>}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
