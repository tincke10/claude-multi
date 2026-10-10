export type BodyResult =
  | { ok: true; bytes: Uint8Array }
  | { ok: false; status: 400 | 413 | 415; error: string };

export interface BodyLimits {
  /** Max bytes accepted on the wire. */
  maxRaw: number;
  /** Max bytes after gzip decompression (zip-bomb guard). */
  maxDecompressed: number;
}

export const LOGS_LIMITS: BodyLimits = { maxRaw: 1024 * 1024, maxDecompressed: 8 * 1024 * 1024 };
export const EVENTS_LIMITS: BodyLimits = { maxRaw: 64 * 1024, maxDecompressed: 64 * 1024 };

const TOO_LARGE: BodyResult = { ok: false, status: 413, error: "payload too large" };

/** Read a stream, aborting as soon as `cap` is exceeded. Returns null when over cap. */
async function readCapped(stream: ReadableStream<Uint8Array>, cap: number): Promise<Uint8Array | null> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > cap) {
        await reader.cancel();
        return null;
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.byteLength;
  }
  return out;
}

export async function readBody(request: Request, limits: BodyLimits): Promise<BodyResult> {
  const encoding = (request.headers.get("content-encoding") ?? "identity").trim().toLowerCase();
  if (encoding !== "identity" && encoding !== "gzip") {
    return { ok: false, status: 415, error: `unsupported content-encoding: ${encoding}` };
  }
  const declared = Number(request.headers.get("content-length"));
  if (Number.isFinite(declared) && declared > limits.maxRaw) return TOO_LARGE;
  if (request.body === null) return { ok: true, bytes: new Uint8Array(0) };

  const raw = await readCapped(request.body, limits.maxRaw);
  if (raw === null) return TOO_LARGE;
  if (encoding === "identity") return { ok: true, bytes: raw };

  try {
    const decompressed = new Blob([raw]).stream().pipeThrough(new DecompressionStream("gzip"));
    const out = await readCapped(decompressed, limits.maxDecompressed);
    return out === null ? TOO_LARGE : { ok: true, bytes: out };
  } catch {
    return { ok: false, status: 400, error: "invalid gzip body" };
  }
}

export function checkContentType(request: Request): { ok: true } | { ok: false; status: 415; error: string } {
  const type = (request.headers.get("content-type") ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
  if (type === "application/json") return { ok: true };
  if (type === "application/x-protobuf") {
    return { ok: false, status: 415, error: "protobuf is not supported; set OTEL_EXPORTER_OTLP_LOGS_PROTOCOL=http/json" };
  }
  return { ok: false, status: 415, error: "content-type must be application/json" };
}
