import { describe, expect, it } from "vitest";
import type { TrackingConfig } from "../api";
import { trackingProblem } from "./tracking";

const t = (u: Partial<TrackingConfig> = {}): TrackingConfig => ({
  enabled: true, ingest_url: "https://i.example.dev", ingest_token: "tok",
  grafana_otlp_endpoint: "", grafana_instance_id: "", grafana_token: "", summaries: false, ...u,
});

describe("trackingProblem", () => {
  it("test_should_allow_anything_when_disabled", () => {
    expect(trackingProblem(t({ enabled: false, ingest_url: "", ingest_token: "" }))).toBeNull();
  });
  it("test_should_accept_valid_config", () => {
    expect(trackingProblem(t())).toBeNull();
  });
  it("test_should_require_url_and_token", () => {
    expect(trackingProblem(t({ ingest_url: "  " }))).toBe("Tracking needs the ingest URL.");
    expect(trackingProblem(t({ ingest_token: "" }))).toBe("Tracking needs the ingest token.");
  });
  it("test_should_require_https_except_for_loopback", () => {
    const msg = "The ingest URL must use https (http is only allowed for localhost).";
    expect(trackingProblem(t({ ingest_url: "http://example.com" }))).toBe(msg);
    expect(trackingProblem(t({ ingest_url: "http://localhost:8787" }))).toBeNull();
    expect(trackingProblem(t({ ingest_url: "http://127.0.0.1" }))).toBeNull();
  });
  it("test_should_require_https_for_grafana_endpoint", () => {
    expect(trackingProblem(t({ grafana_otlp_endpoint: "http://g" }))).toBe("The Grafana endpoint must use https.");
    expect(trackingProblem(t({ grafana_otlp_endpoint: "https://g" }))).toBeNull();
  });
});
