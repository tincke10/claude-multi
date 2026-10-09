import { TrackingConfig } from "../api";

// Client-side mirror of the backend's `tracking::validate_tracking`, so Save
// can be blocked with the same message instead of failing after the click.
// The backend stays the authority; this is only early feedback.
const isLoopbackHttp = (url: string): boolean => {
  const m = /^http:\/\/([^/?#]*)/.exec(url);
  if (!m) return false;
  const host = m[1].replace(/:\d*$/, "");
  return host === "localhost" || host === "127.0.0.1";
};

export function trackingProblem(t: TrackingConfig): string | null {
  if (!t.enabled) return null; // a half-filled disabled config must stay savable
  if (!t.ingest_url.trim()) return "Tracking needs the ingest URL.";
  if (!t.ingest_token) return "Tracking needs the ingest token.";
  if (!t.ingest_url.startsWith("https://") && !isLoopbackHttp(t.ingest_url)) {
    return "The ingest URL must use https (http is only allowed for localhost).";
  }
  if (t.grafana_otlp_endpoint && !t.grafana_otlp_endpoint.startsWith("https://")) {
    return "The Grafana endpoint must use https.";
  }
  return null;
}
