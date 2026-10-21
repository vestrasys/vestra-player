// `.webloc` links (A15): XML and binary property lists, read without system tools.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { binaryPlistUrl, weblocUrl } from "../drop.ts";

// Made once on macOS with `plutil -convert binary1` from the XML shown beside each.
const BINARY = {
  // <dict><key>URL</key><string>https://www.example.com/watch?v=abc&amp;t=10</string></dict>
  ascii:
    "YnBsaXN0MDDRAQJTVVJMXxAoaHR0cHM6Ly93d3cuZXhhbXBsZS5jb20vd2F0Y2g/dj1hYmMmdD0xMAgLDwAAAAAAAAEBAAAAAAAAAAMAAAAAAAAAAAAAAAAAAAA6",
  // <dict><key>Title</key><string>短</string><key>URL</key><string>https://例子.cn/视频</string></dict>
  utf16:
    "YnBsaXN0MDDSAQIDBFVUaXRsZVNVUkxhd+1vEBAAaAB0AHQAcABzADoALwAvT4tbUAAuAGMAbgAvicaYkQgNExcaAAAAAAAAAQEAAAAAAAAABQAAAAAAAAAAAAAAAAAAAD0=",
  // <dict><key>URL</key><string>file:///etc/hosts</string></dict>
  file: "YnBsaXN0MDDRAQJTVVJMXxARZmlsZTovLy9ldGMvaG9zdHMICw8AAAAAAAABAQAAAAAAAAADAAAAAAAAAAAAAAAAAAAAIw==",
};
const bytes = (name: keyof typeof BINARY) => Buffer.from(BINARY[name], "base64");
const xml = (body: string) =>
  `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict>${body}</dict></plist>\n`;

/** A minimal `bplist00`: objects in order, 1-byte offsets and references, top object 0. */
function bplist(...objects: number[][]) {
  const body = [..."bplist00"].map((c) => c.charCodeAt(0));
  const offsets: number[] = [];
  for (const object of objects) {
    offsets.push(body.length);
    body.push(...object);
  }
  const table = body.length;
  body.push(...offsets);
  const trailer = Buffer.alloc(32);
  trailer[6] = 1;
  trailer[7] = 1;
  trailer.writeBigUInt64BE(BigInt(objects.length), 8);
  trailer.writeBigUInt64BE(0n, 16);
  trailer.writeBigUInt64BE(BigInt(table), 24);
  return Buffer.concat([Buffer.from(body), trailer]);
}
const ascii = (text: string) => [0x50 | text.length, ...Buffer.from(text, "latin1")];

test("binary plist: ASCII and UTF-16 URL strings, long lengths, other keys skipped", () => {
  assert.equal(binaryPlistUrl(bytes("ascii")), "https://www.example.com/watch?v=abc&t=10");
  assert.equal(binaryPlistUrl(bytes("utf16")), "https://例子.cn/视频");
  assert.equal(binaryPlistUrl(bytes("file")), "file:///etc/hosts");
  // Hand-made: { URL: "http://a.b" } with a short string, and a UTF-16 value.
  assert.equal(binaryPlistUrl(bplist([0xd1, 1, 2], ascii("URL"), ascii("http://a.b"))), "http://a.b");
  const utf16 = Buffer.from("http://é", "utf16le").swap16();
  assert.equal(binaryPlistUrl(bplist([0xd1, 1, 2], ascii("URL"), [0x68, ...utf16])), "http://é");
});

test("binary plist: no URL key, a non-dictionary top, or malformed data give null", () => {
  assert.equal(binaryPlistUrl(bplist([0xd1, 1, 2], ascii("Title"), ascii("http://a.b"))), null);
  assert.equal(binaryPlistUrl(bplist([0xd1, 1, 2], ascii("URL"), [0x10, 5])), null, "URL is not a string");
  assert.equal(binaryPlistUrl(bplist(ascii("http://a.b"))), null, "top is a string");
  assert.equal(binaryPlistUrl(bplist([0xd1, 1, 9], ascii("URL"))), null, "reference past the objects");
  assert.equal(binaryPlistUrl(bplist([0xd1, 1, 2], ascii("URL"), [0x5f, 0x10, 200])), null, "length past the end");
  const truncated = bytes("ascii").subarray(0, 50);
  assert.equal(binaryPlistUrl(truncated), null);
  assert.equal(binaryPlistUrl(Buffer.from("bplist00")), null);
  assert.equal(binaryPlistUrl(Buffer.from(xml("<key>URL</key><string>http://a.b</string>"))), null, "not binary");
});

test("weblocUrl: XML and binary files; only HTTP(S) links are taken", async () => {
  const root = await mkdtemp(join(tmpdir(), "vestra-webloc-"));
  try {
    const files: [string, string | Buffer, string | null][] = [
      ["xml.webloc", xml("<key>URL</key><string> https://example.org/watch?a=1&amp;b=2&lt;3 </string>"), "https://example.org/watch?a=1&b=2<3"],
      ["xml-file.webloc", xml("<key>URL</key><string>file:///etc/hosts</string>"), null],
      ["xml-none.webloc", xml("<key>Title</key><string>x</string>"), null],
      ["ascii.webloc", bytes("ascii"), "https://www.example.com/watch?v=abc&t=10"],
      ["utf16.webloc", bytes("utf16"), "https://例子.cn/视频"],
      ["file.webloc", bytes("file"), null],
      ["broken.webloc", bytes("ascii").subarray(0, 30), null],
    ];
    for (const [name, content, expected] of files) {
      await writeFile(join(root, name), content);
      assert.equal(await weblocUrl(join(root, name)), expected, name);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
