import { useState } from "react";
import { Config, TrackingCheck, TrackingConfig, testTrackingConnection } from "../api";
import { trackingProblem } from "../lib/tracking";

type Props = {
  config: Config;
  patch: (next: Partial<Config>) => void;
  saveWarnings: string[];
};

export default function TrackingTab({ config: cfg, patch, saveWarnings }: Props) {
  // Last connection test (local: a result for edited values would mislead).
  const [check, setCheck] = useState<TrackingCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkErr, setCheckErr] = useState<string | null>(null);

  const setTracking = (u: Partial<TrackingConfig>) => {
    patch({ tracking: { ...cfg.tracking, ...u } });
    setCheck(null);
    setCheckErr(null);
  };
  const trackingErr = trackingProblem(cfg.tracking);
  const canTest = cfg.tracking.ingest_url.trim() !== "" && cfg.tracking.ingest_token !== "";

  const runCheck = async () => {
    setChecking(true);
    setCheck(null);
    setCheckErr(null);
    try {
      setCheck(await testTrackingConnection(cfg.tracking));
    } catch (e) {
      setCheckErr(`${e}`);
    } finally {
      setChecking(false);
    }
  };

  return (
    <section className="card">
      <div className="card__head">
        <h2 className="card__title">Tracking</h2>
        <label className="check">
          <input
            type="checkbox"
            checked={cfg.tracking.enabled}
            onChange={(e) => setTracking({ enabled: e.target.checked })}
          />
          <span>Enabled</span>
        </label>
      </div>
      <p className="card__hint">
        Opt-in. Reports session activity (counts, tokens, cost — never prompt text) to your own{" "}
        <code>cm-ingest</code> Worker, and metrics to Grafana Cloud. Requires Claude Code 2.1.212 or newer.
      </p>

      <label className="field field--stack">
        <span className="field__label">Ingest URL</span>
        <input
          className="input"
          placeholder="https://cm-ingest.<you>.workers.dev"
          value={cfg.tracking.ingest_url}
          onChange={(e) => setTracking({ ingest_url: e.target.value })}
        />
      </label>
      <label className="field field--stack">
        <span className="field__label">Ingest token</span>
        <input
          className="input"
          type="password"
          autoComplete="off"
          value={cfg.tracking.ingest_token}
          onFocus={(e) => e.target.select()}
          onChange={(e) => setTracking({ ingest_token: e.target.value })}
        />
      </label>

      <p className="card__hint card__hint--sub">Grafana Cloud metrics (optional)</p>
      <label className="field field--stack">
        <span className="field__label">OTLP endpoint</span>
        <input
          className="input"
          placeholder="https://otlp-gateway-<zone>.grafana.net/otlp"
          value={cfg.tracking.grafana_otlp_endpoint}
          onChange={(e) => setTracking({ grafana_otlp_endpoint: e.target.value })}
        />
      </label>
      <div className="row">
        <label className="field">
          <span className="field__label">Instance ID</span>
          <input
            className="input"
            value={cfg.tracking.grafana_instance_id}
            onChange={(e) => setTracking({ grafana_instance_id: e.target.value })}
          />
        </label>
        <label className="field">
          <span className="field__label">Access token</span>
          <input
            className="input"
            type="password"
            autoComplete="off"
            value={cfg.tracking.grafana_token}
            onFocus={(e) => e.target.select()}
            onChange={(e) => setTracking({ grafana_token: e.target.value })}
          />
        </label>
      </div>

      <label className="check check--block">
        <input
          type="checkbox"
          checked={cfg.tracking.summaries}
          onChange={(e) => setTracking({ summaries: e.target.checked })}
        />
        <span>AI summary when a session ends <span className="muted">(runs a small Claude call per session; uses your quota)</span></span>
      </label>

      {trackingErr && <p className="field-error">{trackingErr}</p>}

      <div className="row tracking__test">
        <button className="btn btn--secondary" onClick={runCheck} disabled={!canTest || checking}>
          {checking ? "Testing…" : "Test connection"}
        </button>
        <span className="muted">Uses the values above; saved tokens stay masked.</span>
      </div>
      {checkErr && <p className="field-error">Test failed: {checkErr}</p>}
      {check && (
        <ul className="check-result">
          <li className={check.ingest_ok ? "check-result--ok" : "check-result--err"}>
            Ingest: {check.ingest_message}
          </li>
          {check.grafana_ok !== null && (
            <li className={check.grafana_ok ? "check-result--ok" : "check-result--err"}>
              Grafana: {check.grafana_message}
            </li>
          )}
          {check.warnings.map((w) => (
            <li key={w} className="check-result--warn">{w}</li>
          ))}
        </ul>
      )}
      {saveWarnings.length > 0 && (
        <div className="banner banner--warn">
          <strong>Saved, but:</strong>
          <ul className="banner__list">
            {saveWarnings.map((w) => <li key={w}>{w}</li>)}
          </ul>
        </div>
      )}
    </section>
  );
}
