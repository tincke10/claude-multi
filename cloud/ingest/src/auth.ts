const encoder = new TextEncoder();

async function sha256(text: string): Promise<ArrayBuffer> {
  return crypto.subtle.digest("SHA-256", encoder.encode(text));
}

/**
 * Bearer auth. Both sides are hashed to fixed-size digests first so the
 * constant-time compare leaks neither content nor length.
 */
export async function isAuthorized(request: Request, expectedToken: string): Promise<boolean> {
  if (expectedToken === "") return false;
  const header = request.headers.get("authorization");
  if (header === null) return false;
  const match = /^bearer\s+(.+)$/i.exec(header.trim());
  if (!match || match[1] === undefined) return false;
  const [given, expected] = await Promise.all([sha256(match[1]), sha256(expectedToken)]);
  return crypto.subtle.timingSafeEqual(given, expected);
}
