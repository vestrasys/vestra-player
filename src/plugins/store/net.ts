import { createHash } from "node:crypto";
import { open, rm } from "node:fs/promises";

/**
 * Every request of the store goes through `get` (design §3.2): https only on
 * every hop (plain http only to this machine, for authors testing their own
 * repository), redirects followed by hand (≤ 5), a deadline for the headers
 * and one for the whole transfer, the body counted while it streams and cut
 * off at the limit, and optionally hashed and written to a file.
 */
export type Fetch = (input: string, init: RequestInit) => Promise<Response>;

export type NetErrorCode =
  | "protocol" // the address (or a redirect target) is not https
  | "redirects" // more than MAX_REDIRECTS
  | "http" // a status other than 2xx / 304
  | "tooLarge" // over `maxBytes`
  | "size" // not exactly `exactSize`
  | "sha256" // the digest differs
  | "timeout"
  | "aborted"
  | "network" // DNS, TLS, connection
  | "disk"; // the download file could not be written

export class NetError extends Error {
  readonly code: NetErrorCode;
  readonly status: number | null;
  constructor(code: NetErrorCode, message: string, status: number | null = null) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

export const MAX_REDIRECTS = 5;
const REDIRECT = new Set([301, 302, 303, 307, 308]);
const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** An http address of this machine (allowed for local test repositories). */
export function isLocalHttp(href: string): boolean {
  try {
    const url = new URL(href);
    return url.protocol === "http:" && LOCAL_HOSTS.has(url.hostname);
  } catch {
    return false;
  }
}

export function isHttps(href: string): boolean {
  try {
    return new URL(href).protocol === "https:";
  } catch {
    return false;
  }
}

export interface GetOptions {
  /** Abort the transfer past this many bytes (and refuse a larger Content-Length). */
  maxBytes: number;
  /** The body must be exactly this long (a repository package's `size`). */
  exactSize?: number;
  /** The body's sha256 must be this (lowercase hex). */
  sha256?: string;
  /** Deadline for the whole request. */
  timeoutMs: number;
  /** Deadline for the response headers (default 15 s). */
  headerTimeoutMs?: number;
  headers?: Record<string, string>;
  signal?: AbortSignal;
  /** Write the body here instead of keeping it in memory; removed on failure. */
  file?: string;
  /** Allow http://localhost, 127.0.0.1 and [::1]. */
  allowLocalHttp?: boolean;
  onProgress?: (received: number, total: number | null) => void;
}

export interface Got {
  status: number;
  /** The address after redirects. */
  url: string;
  /** The body; `null` for 304 and when written to `file`. */
  bytes: Buffer | null;
  size: number;
  sha256: string;
  etag: string | null;
  lastModified: string | null;
}

export async function get(fetch: Fetch, url: string, options: GetOptions): Promise<Got> {
  const controller = new AbortController();
  let timedOut = false;
  const expire = () => {
    timedOut = true;
    controller.abort();
  };
  const outer = () => controller.abort();
  if (options.signal?.aborted) throw new NetError("aborted", "已取消");
  options.signal?.addEventListener("abort", outer, { once: true });
  const total = setTimeout(expire, options.timeoutMs);
  const headerTimer = setTimeout(expire, Math.min(options.headerTimeoutMs ?? 15_000, options.timeoutMs));
  const limit = options.exactSize ?? options.maxBytes;
  let fileCreated = false;
  try {
    let current = url;
    for (let hop = 0; ; hop++) {
      const allowed = isHttps(current) || (options.allowLocalHttp === true && isLocalHttp(current));
      if (!allowed)
        throw new NetError("protocol", hop === 0 ? "只支持 https:// 地址" : "下载地址跳转到了不安全的地址");
      const response = await fetch(current, {
        redirect: "manual",
        signal: controller.signal,
        headers: options.headers ?? {},
      });
      if (REDIRECT.has(response.status)) {
        const location = response.headers.get("location");
        await response.body?.cancel().catch(() => {});
        if (!location) throw new NetError("http", `跳转缺少目标地址（${response.status}）`, response.status);
        if (hop >= MAX_REDIRECTS) throw new NetError("redirects", "跳转次数过多");
        current = new URL(location, current).href;
        continue;
      }
      clearTimeout(headerTimer);
      const etag = response.headers.get("etag");
      const lastModified = response.headers.get("last-modified");
      if (response.status === 304) {
        await response.body?.cancel().catch(() => {});
        return { status: 304, url: current, bytes: null, size: 0, sha256: "", etag, lastModified };
      }
      if (!response.ok) {
        await response.body?.cancel().catch(() => {});
        throw new NetError("http", `HTTP ${response.status}`, response.status);
      }
      // With a Content-Encoding (GitHub Pages serves gzip) fetch hands us the
      // decoded body while Content-Length counts the encoded bytes: only the
      // decoded bytes counted below are checked then.
      const encoding = (response.headers.get("content-encoding") ?? "identity").trim().toLowerCase();
      const declared = encoding === "identity" ? response.headers.get("content-length") : null;
      const length = declared === null ? null : Number(declared);
      if (length !== null && Number.isFinite(length)) {
        if (options.exactSize !== undefined && length !== options.exactSize) {
          await response.body?.cancel().catch(() => {});
          throw new NetError("size", "大小与描述不符");
        }
        if (length > options.maxBytes) {
          await response.body?.cancel().catch(() => {});
          throw new NetError("tooLarge", "文件超过大小上限");
        }
      }
      const hash = createHash("sha256");
      const chunks: Buffer[] = [];
      let handle: Awaited<ReturnType<typeof open>> | null = null;
      if (options.file) {
        try {
          handle = await open(options.file, "w");
          fileCreated = true;
        } catch (error) {
          await response.body?.cancel().catch(() => {});
          throw new NetError("disk", (error as Error).message);
        }
      }
      let received = 0;
      try {
        const reader = response.body?.getReader();
        while (reader) {
          const { done, value } = await reader.read();
          if (done) break;
          received += value.byteLength;
          if (received > limit) {
            await reader.cancel().catch(() => {});
            throw options.exactSize !== undefined
              ? new NetError("size", "大小与描述不符")
              : new NetError("tooLarge", "文件超过大小上限");
          }
          const chunk = Buffer.from(value.buffer, value.byteOffset, value.byteLength);
          hash.update(chunk);
          if (handle) {
            try {
              await handle.write(chunk);
            } catch (error) {
              await reader.cancel().catch(() => {});
              throw new NetError("disk", (error as Error).message);
            }
          } else chunks.push(Buffer.from(chunk));
          options.onProgress?.(received, length);
        }
      } finally {
        await handle?.close().catch(() => {});
      }
      if (options.exactSize !== undefined && received !== options.exactSize)
        throw new NetError("size", "大小与描述不符");
      const digest = hash.digest("hex");
      if (options.sha256 !== undefined && digest !== options.sha256)
        throw new NetError("sha256", "校验和与描述不符");
      return {
        status: response.status,
        url: current,
        bytes: handle ? null : Buffer.concat(chunks),
        size: received,
        sha256: digest,
        etag,
        lastModified,
      };
    }
  } catch (error) {
    if (fileCreated && options.file) await rm(options.file, { force: true }).catch(() => {});
    if (error instanceof NetError) throw error;
    if (controller.signal.aborted)
      throw timedOut ? new NetError("timeout", "连接超时") : new NetError("aborted", "已取消");
    throw new NetError("network", networkReason(error));
  } finally {
    clearTimeout(total);
    clearTimeout(headerTimer);
    options.signal?.removeEventListener("abort", outer);
  }
}

/** `fetch failed` hides the reason in `cause` (ENOTFOUND, ECONNREFUSED, a TLS error). */
function networkReason(error: unknown): string {
  const cause = (error as { cause?: { code?: string; message?: string } }).cause;
  if (cause?.code === "ENOTFOUND" || cause?.code === "EAI_AGAIN") return "找不到服务器";
  if (cause?.code === "ECONNREFUSED") return "连接被拒绝";
  if (cause?.code === "ECONNRESET") return "连接被重置";
  if (cause?.code && /CERT|SSL|TLS/i.test(cause.code)) return "安全连接失败";
  return cause?.message || (error as Error).message || String(error);
}
