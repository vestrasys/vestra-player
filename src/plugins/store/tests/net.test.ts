import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { get, NetError } from "../net.ts";
import { FakeNet, sha256 } from "./helpers.ts";

const scratch = mkdtempSync(join(tmpdir(), "vestra-store-net-"));
test.after(() => rmSync(scratch, { recursive: true, force: true }));

const code = (expected: string) => (error: unknown) => error instanceof NetError && error.code === expected;
const body = Buffer.from("x".repeat(1000));

test("exact size and sha256 pass; one byte more or a wrong digest fails and leaves no file", async () => {
  const net = new FakeNet();
  net.route("https://d.test/a.vxp", { chunks: [body.subarray(0, 400), body.subarray(400)] });
  const file = join(scratch, "a.part");
  const got = await get(net.fetch, "https://d.test/a.vxp", {
    maxBytes: 1000,
    exactSize: 1000,
    sha256: sha256(body),
    timeoutMs: 1000,
    file,
  });
  assert.equal(got.size, 1000);
  assert.deepEqual(readFileSync(file), body);
  rmSync(file);

  net.route("https://d.test/big.vxp", { chunks: [body, Buffer.from("y")], noLength: true });
  await assert.rejects(get(net.fetch, "https://d.test/big.vxp", { maxBytes: 1000, exactSize: 1000, timeoutMs: 1000, file }), code("size"));
  assert.ok(!existsSync(file), "partial file removed");
  await assert.rejects(get(net.fetch, "https://d.test/a.vxp", { maxBytes: 999, exactSize: 999, timeoutMs: 1000, file }), code("size"));
  net.route("https://d.test/short.vxp", { body: body.subarray(0, 10), noLength: true });
  await assert.rejects(get(net.fetch, "https://d.test/short.vxp", { maxBytes: 1000, exactSize: 1000, timeoutMs: 1000 }), code("size"));
  await assert.rejects(
    get(net.fetch, "https://d.test/a.vxp", { maxBytes: 1000, exactSize: 1000, sha256: "0".repeat(64), timeoutMs: 1000, file }),
    code("sha256"),
  );
  assert.ok(!existsSync(file));
});

test("a Content-Length over the limit is refused before reading; an unsized body is cut off", async () => {
  const net = new FakeNet();
  net.route("https://d.test/huge", { headers: { "content-length": "999999999" }, body: "small" });
  await assert.rejects(get(net.fetch, "https://d.test/huge", { maxBytes: 4096, timeoutMs: 1000 }), code("tooLarge"));
  net.route("https://d.test/stream", { chunks: Array.from({ length: 10 }, () => Buffer.alloc(1000)), noLength: true });
  await assert.rejects(get(net.fetch, "https://d.test/stream", { maxBytes: 4096, timeoutMs: 1000 }), code("tooLarge"));
});

test("https only on every hop; at most 5 redirects", async () => {
  const net = new FakeNet();
  await assert.rejects(get(net.fetch, "http://d.test/x", { maxBytes: 10, timeoutMs: 1000 }), code("protocol"));
  await assert.rejects(get(net.fetch, "file:///etc/passwd", { maxBytes: 10, timeoutMs: 1000 }), code("protocol"));
  net.route("https://d.test/r0", { status: 302, headers: { location: "https://cdn.test/r1" } });
  net.route("https://cdn.test/r1", { status: 301, headers: { location: "/final" } });
  net.route("https://cdn.test/final", { body: "ok" });
  const got = await get(net.fetch, "https://d.test/r0", { maxBytes: 10, timeoutMs: 1000 });
  assert.equal(got.bytes!.toString(), "ok");
  assert.equal(got.url, "https://cdn.test/final");
  net.route("https://d.test/down", { status: 302, headers: { location: "http://d.test/plain" } });
  await assert.rejects(get(net.fetch, "https://d.test/down", { maxBytes: 10, timeoutMs: 1000 }), code("protocol"));
  assert.equal(net.count("http://d.test/plain"), 0, "the http hop is never requested");
  for (let i = 0; i < 7; i++) net.route(`https://d.test/loop${i}`, { status: 307, headers: { location: `https://d.test/loop${i + 1}` } });
  net.route("https://d.test/loop7", { body: "too far" });
  await assert.rejects(get(net.fetch, "https://d.test/loop0", { maxBytes: 10, timeoutMs: 1000 }), code("redirects"));
  // Five redirects are fine.
  net.route("https://d.test/loop5", { body: "five" });
  assert.equal((await get(net.fetch, "https://d.test/loop0", { maxBytes: 10, timeoutMs: 1000 })).bytes!.toString(), "five");
});

test("HTTP errors, 304, time-outs, cancel and offline", async () => {
  const net = new FakeNet();
  await assert.rejects(get(net.fetch, "https://d.test/none", { maxBytes: 10, timeoutMs: 1000 }), (e: NetError) => e.code === "http" && e.status === 404);
  net.route("https://d.test/same", { status: 304 });
  assert.equal((await get(net.fetch, "https://d.test/same", { maxBytes: 10, timeoutMs: 1000 })).status, 304);
  net.route("https://d.test/slow", { delay: 500, body: "late" });
  await assert.rejects(get(net.fetch, "https://d.test/slow", { maxBytes: 10, timeoutMs: 50 }), code("timeout"));
  await assert.rejects(get(net.fetch, "https://d.test/slow", { maxBytes: 10, timeoutMs: 1000, headerTimeoutMs: 30 }), code("timeout"));
  const controller = new AbortController();
  setTimeout(() => controller.abort(), 20);
  await assert.rejects(get(net.fetch, "https://d.test/slow", { maxBytes: 10, timeoutMs: 1000, signal: controller.signal }), code("aborted"));
  net.offline = true;
  await assert.rejects(get(net.fetch, "https://d.test/same", { maxBytes: 10, timeoutMs: 1000 }), (e: NetError) => e.code === "network" && e.message === "找不到服务器");
});

test("real fetch against a server on 127.0.0.1: local http allowed only when asked, streaming cut off", async () => {
  const server = createServer((request, response) => {
    if (request.url === "/endless") {
      response.writeHead(200);
      const timer = setInterval(() => response.write(Buffer.alloc(16 * 1024)), 1);
      response.on("close", () => clearInterval(timer));
      return;
    }
    if (request.url === "/hop") {
      response.writeHead(302, { location: "/index.json" });
      return response.end();
    }
    response.writeHead(200, { "content-type": "application/json" });
    response.end('{"schema":1}');
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as AddressInfo).port;
  const fetch = (input: string, init: RequestInit) => globalThis.fetch(input, init);
  try {
    const url = `http://127.0.0.1:${port}/index.json`;
    await assert.rejects(get(fetch, url, { maxBytes: 100, timeoutMs: 2000 }), code("protocol"));
    const got = await get(fetch, `http://127.0.0.1:${port}/hop`, { maxBytes: 100, timeoutMs: 2000, allowLocalHttp: true });
    assert.equal(got.bytes!.toString(), '{"schema":1}');
    const file = join(scratch, "endless.part");
    await assert.rejects(
      get(fetch, `http://127.0.0.1:${port}/endless`, { maxBytes: 256 * 1024, timeoutMs: 5000, allowLocalHttp: true, file }),
      code("tooLarge"),
    );
    assert.ok(!readdirSync(scratch).includes("endless.part"));
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
});
