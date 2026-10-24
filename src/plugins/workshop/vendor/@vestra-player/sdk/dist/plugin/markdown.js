/**
 * `plugin.ui.markdown`: the player parses and draws the document; this side
 * downloads its `https:` images into the plugin's data directory first
 * (Node's `fetch`, so the player needs no HTTP stack). The cache is
 * `<dataDirectory>/.vestra-cache/markdown/`, one file per address.
 */
import { createHash } from "node:crypto";
import { mkdir, readdir, rename, stat, unlink, utimes, writeFile, } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { failure } from "./runtime.js";
export const MARKDOWN_LIMITS = Object.freeze({
    /** Text bytes (UTF-8). */
    textBytes: 256 * 1024,
    /** Bytes of one downloaded image. */
    imageBytes: 4 * 1024 * 1024,
    /** Download time per image. */
    imageTimeoutMs: 10_000,
    /** Redirects followed per image. */
    redirects: 5,
    /** Downloads at once, per plugin. */
    concurrency: 4,
    /** Cache size per plugin. */
    cacheBytes: 32 * 1024 * 1024,
    /** An address is fetched again after this long. */
    cacheMaxAgeMs: 24 * 60 * 60 * 1000,
});
let documents = 0;
/** The wire form (`{ $vestraMarkdown: { text, base, images }, ref }`) is internal. */
export class MarkdownHandle {
    text;
    __vestraMarkdown = true;
    #scope;
    #base;
    #images;
    #ref = ++documents;
    constructor(scope, text, base, images) {
        this.#scope = scope;
        this.text = text;
        this.#base = base;
        this.#images = images;
    }
    /** Released with its scope: writing it is `invalid`. */
    get released() {
        return this.#scope.disposed;
    }
    toJSON() {
        return {
            $vestraMarkdown: {
                text: this.text,
                base: this.#base,
                images: { ...this.#images },
            },
            ref: this.#ref,
        };
    }
}
const isPng = (b) => b.length > 8 &&
    b[0] === 0x89 &&
    b[1] === 0x50 &&
    b[2] === 0x4e &&
    b[3] === 0x47;
const isJpeg = (b) => b.length > 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff;
/** Sent with every image download (some hosts refuse anonymous clients). */
export const USER_AGENT = "VestraPlayer-Markdown/1.1 (+https://github.com/vestrasys/vestra-player-sdk)";
/**
 * `https:` only, redirects followed by hand (≤ 5, all `https:`), size capped
 * while reading; an HTTP error status throws. `fetchImpl` is for tests.
 */
export async function httpsDownload(url, options, fetchImpl = fetch) {
    let address = url;
    for (let hop = 0; hop <= MARKDOWN_LIMITS.redirects; hop++) {
        if (!address.startsWith("https://"))
            throw new Error("only https: images are downloaded");
        const response = await fetchImpl(address, {
            redirect: "manual",
            signal: options.signal,
            credentials: "omit",
            headers: { "user-agent": USER_AGENT },
        });
        if (response.status >= 300 && response.status < 400) {
            const location = response.headers.get("location");
            await response.body?.cancel();
            if (!location)
                throw new Error(`redirect without location`);
            address = new URL(location, address).toString();
            continue;
        }
        if (!response.ok) {
            await response.body?.cancel();
            throw new Error(`HTTP ${response.status}`);
        }
        const length = Number(response.headers.get("content-length"));
        if (length > options.maxBytes) {
            await response.body?.cancel();
            throw new Error("image too large");
        }
        const chunks = [];
        let total = 0;
        for await (const chunk of response.body ?? []) {
            total += chunk.length;
            if (total > options.maxBytes)
                throw new Error("image too large");
            chunks.push(chunk);
        }
        return Buffer.concat(chunks);
    }
    throw new Error("too many redirects");
}
/** Download (or reuse) the images; address → local file for each success. */
async function fetchImages(runtime, urls) {
    const data = runtime.dataDirectory;
    if (!data || !urls.length)
        return {};
    const cache = join(data, ".vestra-cache", "markdown");
    await mkdir(cache, { recursive: true });
    const download = runtime.local?.download ?? httpsDownload;
    const found = {};
    const queue = [...new Set(urls)].filter((u) => u.startsWith("https://"));
    const worker = async () => {
        for (let url = queue.shift(); url !== undefined; url = queue.shift()) {
            const file = join(cache, createHash("sha256").update(url).digest("hex"));
            try {
                const info = await stat(file).catch(() => null);
                const now = Date.now();
                if (info && now - info.mtimeMs < MARKDOWN_LIMITS.cacheMaxAgeMs) {
                    const time = new Date(info.mtimeMs);
                    await utimes(file, new Date(now), time).catch(() => { });
                    found[url] = file;
                    continue;
                }
                const bytes = await download(url, {
                    maxBytes: MARKDOWN_LIMITS.imageBytes,
                    signal: AbortSignal.timeout(MARKDOWN_LIMITS.imageTimeoutMs),
                });
                if (bytes.length > MARKDOWN_LIMITS.imageBytes)
                    throw new Error("image too large");
                if (!isPng(bytes) && !isJpeg(bytes))
                    throw new Error("not a PNG or JPEG image");
                const temporary = `${file}.${process.pid}.tmp`;
                await writeFile(temporary, bytes);
                await rename(temporary, file);
                found[url] = file;
            }
            catch {
                // Shown as its alt text.
            }
        }
    };
    await Promise.all(Array.from({ length: MARKDOWN_LIMITS.concurrency }, worker));
    await trim(cache, new Set(Object.values(found)));
    return found;
}
/** Keep the cache under its size: drop the least recently used files not in use. */
async function trim(cache, keep) {
    const files = [];
    for (const name of await readdir(cache).catch(() => [])) {
        const path = join(cache, name);
        const info = await stat(path).catch(() => null);
        if (info?.isFile())
            files.push({ path, size: info.size, used: info.atimeMs });
    }
    let total = files.reduce((sum, f) => sum + f.size, 0);
    files.sort((a, b) => a.used - b.used);
    for (const file of files) {
        if (total <= MARKDOWN_LIMITS.cacheBytes)
            break;
        if (keep.has(file.path))
            continue;
        await unlink(file.path).catch(() => { });
        total -= file.size;
    }
}
export async function markdown(scope, text, options = {}) {
    if (typeof text !== "string")
        throw failure("invalid", "ui.markdown needs the Markdown text");
    if (Buffer.byteLength(text) > MARKDOWN_LIMITS.textBytes)
        throw failure("invalid", "Markdown text exceeds 256 KB");
    const { base, remoteImages } = options ?? {};
    if (base !== undefined &&
        (typeof base !== "string" ||
            !(/^https?:\/\//i.test(base) || isAbsolute(base))))
        throw failure("invalid", "base must be an http(s) address or an absolute directory");
    let images = {};
    if (remoteImages !== false) {
        const scan = await scope.runtime.host.ui.markdownScan(text, base ?? null);
        images = await fetchImages(scope.runtime, scan.images);
    }
    return new MarkdownHandle(scope, text, base ?? null, images);
}
