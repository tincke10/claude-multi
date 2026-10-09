import { describe, expect, it } from "vitest";
import { checkContentType, readBody } from "../src/body";

const LIM = { maxRaw: 100, maxDecompressed: 1000 };
const enc = new TextEncoder();
const dec = new TextDecoder();

async function gzip(text: string): Promise<Uint8Array> {
  const stream = new Blob([enc.encode(text)]).stream().pipeThrough(new CompressionStream("gzip"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}
function post(body: BodyInit | null, headers: Record<string, string> = {}) {
  return new Request("https://x/", { method: "POST", body, headers });
}
function streamOf(...chunks: Uint8Array[]): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(c) {
      for (const ch of chunks) c.enqueue(ch);
      c.close();
    },
  });
}

describe("readBody", () => {
  it("test_should_return_bytes_when_under_cap", async () => {
    const r = await readBody(post("hello"), LIM);
    expect(r.ok && dec.decode(r.bytes)).toBe("hello");
  });

  it("test_should_return_413_when_content_length_over_cap", async () => {
    const r = await readBody(post("x", { "content-length": "101" }), LIM);
    expect(r).toMatchObject({ ok: false, status: 413 });
  });

  it("test_should_return_413_when_streamed_body_exceeds_cap_without_content_length", async () => {
    const req = new Request("https://x/", {
      method: "POST",
      body: streamOf(enc.encode("a".repeat(60)), enc.encode("a".repeat(60))),
      // @ts-expect-error duplex is required by the spec for stream bodies but missing from the types
      duplex: "half",
    });
    expect(req.headers.get("content-length")).toBeNull();
    expect(await readBody(req, LIM)).toMatchObject({ ok: false, status: 413 });
  });

  it("test_should_decompress_when_gzip", async () => {
    const r = await readBody(post(await gzip("compressed hello"), { "content-encoding": "gzip" }), LIM);
    expect(r.ok && dec.decode(r.bytes)).toBe("compressed hello");
  });

  it("test_should_return_413_when_decompressed_exceeds_cap", async () => {
    const bomb = await gzip("a".repeat(5000));
    expect(bomb.byteLength).toBeLessThan(100);
    const r = await readBody(post(bomb, { "content-encoding": "gzip" }), LIM);
    expect(r).toMatchObject({ ok: false, status: 413 });
  });

  it("test_should_return_400_when_gzip_is_corrupt", async () => {
    const r = await readBody(post(enc.encode("not gzip at all"), { "content-encoding": "gzip" }), LIM);
    expect(r).toMatchObject({ ok: false, status: 400 });
  });

  it("test_should_return_415_when_encoding_unsupported", async () => {
    const r = await readBody(post("x", { "content-encoding": "br" }), LIM);
    expect(r).toMatchObject({ ok: false, status: 415 });
  });

  it("test_should_accept_when_encoding_identity", async () => {
    const r = await readBody(post("hi", { "content-encoding": "identity" }), LIM);
    expect(r.ok).toBe(true);
  });

  it("test_should_return_empty_bytes_when_no_body", async () => {
    const r = await readBody(post(null), LIM);
    expect(r.ok && r.bytes.byteLength).toBe(0);
  });
});

describe("checkContentType", () => {
  const ct = (v?: string) => post("{}", v === undefined ? {} : { "content-type": v });
  it("test_should_accept_when_json", () => {
    expect(checkContentType(ct("application/json")).ok).toBe(true);
    expect(checkContentType(ct("application/json; charset=utf-8")).ok).toBe(true);
  });
  it("test_should_hint_http_json_when_protobuf", () => {
    const r = checkContentType(ct("application/x-protobuf"));
    expect(r).toMatchObject({ ok: false, status: 415 });
    expect(!r.ok && r.error).toContain("http/json");
  });
  it("test_should_reject_when_not_json", () => {
    expect(checkContentType(ct("text/plain")).ok).toBe(false);
  });
});
