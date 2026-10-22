import { createPublicKey, verify, type KeyObject } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * The official repository's index signing key: `keys/index-signing.pub.pem`,
 * a verbatim copy of the plugins repository's file (SPKI PEM, Ed25519). It
 * ships inside this package; there is no setting or environment variable that
 * replaces it. Tests pass their own key list to the store instead.
 */
export const OFFICIAL_KEY_FILE = new URL("./keys/index-signing.pub.pem", import.meta.url);

export const OFFICIAL_KEYS_DIRECTORY = new URL("./keys/", import.meta.url);

/**
 * Every `keys/*.pub.pem` in the package (the plugins repository's keys; two
 * during a key rotation). A signature that verifies with any of them counts,
 * so a rotation needs only a store release that adds the next key.
 */
export function officialKeys(directory: string | URL = OFFICIAL_KEYS_DIRECTORY): KeyObject[] {
  const root = typeof directory === "string" ? directory : fileURLToPath(directory);
  const keys = readdirSync(root)
    .filter((name) => name.endsWith(".pub.pem"))
    .sort()
    .map((name) => {
      const key = createPublicKey({ key: readFileSync(join(root, name), "utf8"), format: "pem" });
      if (key.asymmetricKeyType !== "ed25519") throw new Error(`${name} is not an Ed25519 key`);
      return key;
    });
  if (!keys.length) throw new Error("no official index key");
  return keys;
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
