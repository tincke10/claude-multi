// GRAFANA_AUTH is an OPTIONAL secret, so `wrangler types` (secrets.required) cannot
// declare it. It is the full Authorization header value for the Grafana OTLP
// logs endpoint (e.g. "Basic <base64 instanceId:token>"). Forwarding is disabled
// when it or GRAFANA_OTLP_LOGS_URL is empty.
interface Env {
  GRAFANA_AUTH?: string;
}
declare namespace Cloudflare {
  interface Env {
    GRAFANA_AUTH?: string;
  }
}
