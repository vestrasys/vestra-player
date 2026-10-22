import { createPublicKey, verify, type KeyObject } from "node:crypto";
import { readFileSync } from "node:fs";

/**
 * The official repository's index signing key: `keys/index-signing.pub.pem`,
 * a verbatim copy of the plugins repository's file (SPKI PEM, Ed25519). It
 * ships inside this package; there is no setting or environment variable that
 * replaces it. Tests pass their own key list to the store instead.
 */
export const OFFICIAL_KEY_FILE = new URL("./keys/index-signing.pub.pem", import.meta.url);

export function officialKeys(): KeyObject[] {
  const key = createPublicKey({ key: readFileSync(OFFICIAL_KEY_FILE, "utf8"), format: "pem" });
  if (key.asymmetricKeyType !== "ed25519") throw new Error("the official index key is not Ed25519");
  return [key];
}

const BASE64 = /^[A-Za-z0-9+/]+={0,2}$/;

/**
 * Whether `signature` (the text of index.json.sig) signs exactly `bytes`.
 * The file may hold several base64 signatures separated by white space (room
 * for a key rotation); one that verifies with any trusted key is enough.
 */
export function verifyIndex(bytes: Uint8Array, signature: string, keys: readonly KeyObject[]): boolean {
  return signature
    .trim()
    .split(/\s+/)
    .some((part) => {
      if (!BASE64.test(part)) return false;
      const raw = Buffer.from(part, "base64");
      return raw.length === 64 && keys.some((key) => verify(null, bytes, key, raw));
    });
}
