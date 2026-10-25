import assert from "node:assert/strict";
import { createHash, createPublicKey } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";
import { bestVersion, checkVersion, revocationOf } from "../catalog.ts";
import { clean, parseIndex, remember, replayed, IndexError } from "../index-format.ts";
import { permissionList, permissions } from "../labels.ts";
import { msg, type Many } from "../text.ts";
import { normalizeRepoUrl } from "../repos.ts";
import { compareVersions, parseRange, playerAtLeast, satisfies } from "../semver.ts";
import { OFFICIAL_KEY_FILE, officialKeys, verifyIndex } from "../signing.ts";
import { LIVE_INDEX, LIVE_SIGNATURE, OFFICIAL_KEY_FINGERPRINT } from "./fixtures.ts";
import { buildIndex, FakeNet, other, pair, signWith } from "./helpers.ts";

test("version ranges: the plugins repository's cases", () => {
  assert.ok(compareVersions("1.10.0", "1.9.9") > 0);
  assert.ok(satisfies("1.2.0", "1.2.0"));
  assert.ok(!satisfies("1.2.1", "1.2.0"));
  assert.ok(satisfies("1.1.5", ">=1.0.0 <1.2.0"));
  assert.ok(!satisfies("1.2.0", ">=1.0.0 <1.2.0"));
  assert.ok(satisfies("2.0.0", ">=1.0.0 <1.2.0 || 2.0.0"));
  assert.ok(satisfies("9.9.9", "*"));
  assert.ok(satisfies("0.9.0", "<=0.9.0") && satisfies("1.0.1", ">1.0.0") && satisfies("1.0.0", "=1.0.0"));
  for (const bad of ["", "^1.0.0", "~1.2.0", "1.x", "1.0", "01.0.0", "1.0.0 - 2.0.0", ">= 1.0.0", "1.0.0-beta", "||"])
    assert.throws(() => parseRange(bad), Error, bad);
});

test("player version: three leading numbers, suffix ignored", () => {
  assert.ok(playerAtLeast("0.2.0", "0.2.0"));
  assert.ok(playerAtLeast("0.2.1-dev", "0.2.0"));
  assert.ok(!playerAtLeast("0.2.0", "0.10.0"));
  assert.ok(!playerAtLeast("0.2.0", "garbage"));
});

test("the embedded key is the plugins repository's key and verifies the live index", () => {
  const der = createPublicKey(readFileSync(OFFICIAL_KEY_FILE, "utf8")).export({ type: "spki", format: "der" });
  assert.equal(createHash("sha256").update(der).digest("hex"), OFFICIAL_KEY_FINGERPRINT);
  const keys = officialKeys();
  assert.ok(verifyIndex(LIVE_INDEX, LIVE_SIGNATURE, keys));
  const tampered = Buffer.from(LIVE_INDEX);
  tampered[100] = tampered[100]! ^ 1;
  assert.ok(!verifyIndex(tampered, LIVE_SIGNATURE, keys));
  // The live data parses and selects a version for player 0.2.0.
  const { index, dropped } = parseIndex(LIVE_INDEX.toString("utf8"));
  assert.deepEqual(dropped, []);
  assert.deepEqual(
    index.plugins.map((p) => [p.id, bestVersion(p, index.revoked, "0.2.0")?.version]),
    [
      ["zysilm.bilibili", "0.3.0"],
      ["zysilm.youtube", "0.3.0"],
    ],
  );
});

test("signatures: test key pair, tampering, wrong key, white space, several signatures", () => {
  const bytes = Buffer.from('{"schema":1}\n');
  const good = signWith(bytes);
  const keys = [pair.publicKey];
  assert.ok(verifyIndex(bytes, good, keys));
  assert.ok(verifyIndex(bytes, `  ${good}\r\n`, keys));
  assert.ok(!verifyIndex(Buffer.from('{"schema":2}\n'), good, keys), "one changed byte");
  assert.ok(!verifyIndex(bytes, signWith(bytes, other.privateKey), keys), "another key");
  assert.ok(!verifyIndex(bytes, "not base64!", keys));
  assert.ok(!verifyIndex(bytes, Buffer.alloc(32).toString("base64"), keys), "not 64 bytes");
  assert.ok(!verifyIndex(bytes, "", keys));
  assert.ok(verifyIndex(bytes, `${signWith(bytes, other.privateKey)}\n${good}\n`, keys), "one of two is enough");
  assert.ok(!verifyIndex(bytes, `${signWith(bytes, other.privateKey)} ${signWith(bytes, other.privateKey)}`, keys));
  assert.ok(verifyIndex(bytes, signWith(bytes, other.privateKey), [pair.publicKey, other.publicKey]), "key rotation");
});

test("index parsing: lenient per entry, strict at the top", () => {
  const net = new FakeNet();
  const text = buildIndex(net, "https://r.test/index.json", [
    { id: "a.one", versions: [{ version: "1.0.0" }, { version: "1.2.0" }] },
    { id: "b.two", versions: [{ version: "0.1.0" }] },
  ]).toString("utf8");
  const data = JSON.parse(text);
  data.future = { anything: true };
  data.plugins[0].newField = 1;
  data.plugins[0].name = "x".repeat(100);
  data.plugins.push({ id: "bad id", versions: [] });
  data.plugins.push({ ...data.plugins[1], id: "B.TWO" });
  data.plugins.push({ ...data.plugins[1], id: "c.http", versions: [{ ...data.plugins[1].versions[0], downloadUrl: "http://x.test/c.vxp" }] });
  data.plugins[1].versions.push({ ...data.plugins[1].versions[0], version: "0.2.0", sha256: "XYZ" });
  data.revoked = [
    { id: "a.one", versions: "<1.1.0", reason: "bug" },
    { id: "a.one", versions: "^1.0.0", reason: "bad range" },
    { id: "a.one", versions: "1.0.0", reason: "" },
  ];
  const { index, dropped } = parseIndex(JSON.stringify(data));
  assert.deepEqual(index.plugins.map((p) => p.id), ["a.one", "b.two"]);
  assert.equal([...index.plugins[0]!.name].length, 64);
  assert.deepEqual(index.plugins[0]!.versions.map((v) => v.version), ["1.2.0", "1.0.0"], "sorted newest first");
  assert.deepEqual(index.plugins[1]!.versions.map((v) => v.version), ["0.1.0"], "bad sha256 dropped");
  assert.deepEqual(index.revoked, [{ id: "a.one", versions: "<1.1.0", reason: "bug" }]);
  assert.ok(dropped.length >= 5);
  // http download links are fine only for a repository on this machine.
  const local = parseIndex(JSON.stringify({ ...data, plugins: [data.plugins.at(-1)] }), { allowLocalHttp: true });
  assert.equal(local.index.plugins.length, 0, "x.test is not this machine");
  assert.throws(() => parseIndex("{"), IndexError);
  assert.throws(() => parseIndex(JSON.stringify({ ...data, schema: 2 })), (e: IndexError) => e.kind === "schema");
  assert.throws(() => parseIndex(JSON.stringify({ ...data, plugins: {} })), (e: IndexError) => e.kind === "format");
});

test("replay check: seen versions and revocations never shrink; a delisted plugin may be absent", () => {
  const net = new FakeNet();
  const parse = (versions: string[], extra = true, revoked: { id: string; versions: string; reason: string }[] = []) =>
    parseIndex(
      buildIndex(
        net,
        "https://r.test/index.json",
        [
          ...(versions.length ? [{ id: "a.one", versions: versions.map((version) => ({ version })) }] : []),
          ...(extra ? [{ id: "b.two", versions: [{ version: "1.0.0" }] }] : []),
        ],
        revoked,
      ).toString("utf8"),
    ).index;
  const seen = remember(undefined, parse(["1.0.0", "1.1.0"]));
  assert.equal(replayed(seen, parse(["1.0.0", "1.1.0", "1.2.0"])), null);
  assert.equal(replayed(seen, parse(["1.0.0"])), "a.one 1.1.0");
  assert.equal(replayed(seen, parse(["1.0.0", "1.1.0"], false)), null, "b.two delisted");
  assert.equal(replayed(undefined, parse(["1.0.0"])), null);
  // Scenario C: a.one absent for a while keeps its seen versions.
  const absent = remember(seen, parse([]));
  assert.deepEqual(absent.versions["a.one"]?.sort(), ["1.0.0", "1.1.0"]);
  assert.equal(replayed(absent, parse(["1.0.0"])), "a.one 1.1.0");
  // Scenario B: a revocation once seen must stay.
  const revoked = [{ id: "a.one", versions: "1.1.0", reason: "bad" }];
  const withRevocation = remember(seen, parse(["1.0.0", "1.1.0"], true, revoked));
  assert.equal(replayed(withRevocation, parse(["1.0.0", "1.1.0"])), "revoked a.one 1.1.0");
  assert.equal(replayed(withRevocation, parse(["1.0.0", "1.1.0", "1.2.0"], true, revoked)), null);
});

test("invisible characters are removed from display text; too many revocations reject the index", () => {
  assert.equal(clean("Vestra\u202e\u200bPlayer\ufeff\u2066x\u2069", 64), "VestraPlayerx");
  const net = new FakeNet();
  const many = Array.from({ length: 1001 }, (_, i) => ({ id: `p.${i}`, versions: "*", reason: "x" }));
  const text = buildIndex(net, "https://r.test/index.json", [], many).toString("utf8");
  assert.throws(() => parseIndex(text), (e: IndexError) => e.kind === "format" && /revoked/.test(e.message));
  assert.equal(parseIndex(buildIndex(net, "https://r.test/index.json", [], many.slice(0, 1000)).toString("utf8")).index.revoked.length, 1000);
});

test("version choice: API, player version and revocations", () => {
  const net = new FakeNet();
  const { index } = parseIndex(
    buildIndex(net, "https://r.test/index.json", [
      {
        id: "a.one",
        versions: [
          { version: "3.0.0", apiVersion: "vestra/2" },
          { version: "2.0.0", minPlayerVersion: "9.0.0" },
          { version: "1.2.0" },
          { version: "1.1.0" },
          { version: "1.0.0", minPlayerVersion: null },
        ],
      },
    ]).toString("utf8"),
  );
  const entry = index.plugins[0]!;
  assert.equal(bestVersion(entry, [], "0.2.0")?.version, "1.2.0");
  assert.equal(bestVersion(entry, [], "9.0.0")?.version, "2.0.0");
  const revoked = [{ id: "a.one", versions: ">=1.1.0 <1.3.0", reason: "数据丢失" }];
  assert.equal(bestVersion(entry, revoked, "0.2.0")?.version, "1.0.0");
  assert.equal(bestVersion(entry, [{ id: "a.one", versions: "*", reason: "x" }], "0.2.0"), null);
  assert.equal(revocationOf("a.one", "1.2.0", revoked)?.reason, "数据丢失");
  assert.equal(revocationOf("other", "1.2.0", revoked), null);
  const v3 = entry.versions[0]!;
  assert.deepEqual(checkVersion("a.one", v3, [], "0.2.0"), { ok: false, why: "api", text: msg("Needs a newer plugin API ({})", "vestra/2") });
  const v2 = entry.versions[1]!;
  assert.deepEqual(checkVersion("a.one", v2, [], "0.2.0"), {
    ok: false,
    why: "player",
    text: msg("Needs Vestra Player ≥ {} (current {})", "9.0.0", "0.2.0"),
  });
});

test("repository addresses are normalized; only https (and this machine's http)", () => {
  assert.deepEqual(normalizeRepoUrl(" https://Foo.example/plugins#x "), { url: "https://foo.example/plugins/index.json", local: false });
  assert.deepEqual(normalizeRepoUrl("https://foo.example/plugins/"), { url: "https://foo.example/plugins/index.json", local: false });
  assert.deepEqual(normalizeRepoUrl("https://foo.example/a/list.json"), { url: "https://foo.example/a/list.json", local: false });
  assert.deepEqual(normalizeRepoUrl("http://127.0.0.1:8080/"), { url: "http://127.0.0.1:8080/index.json", local: true });
  assert.deepEqual(normalizeRepoUrl("http://localhost:8080/r"), { url: "http://localhost:8080/r/index.json", local: true });
  assert.ok("error" in normalizeRepoUrl("http://foo.example/"));
  assert.ok("error" in normalizeRepoUrl("ftp://foo.example/"));
  assert.ok("error" in normalizeRepoUrl("not a url"));
  assert.ok("error" in normalizeRepoUrl("https://user:pw@foo.example/"));
});

test("permissions in plain words; unknown ones are named; long lists are cut", () => {
  assert.deepEqual(permissions(["clipboard", "media.read", "media.control", "future.thing"]), [
    { id: "clipboard", text: msg("Read and write the clipboard"), risky: true },
    { id: "media.read", text: msg("Take snapshots and export clips"), risky: false },
    { id: "future.thing", text: msg("Unknown permission: {}", "future.thing"), risky: false },
  ]);
  const many = Array.from({ length: 25 }, (_, i) => `x.${i}`);
  const lines = permissionList(many) as Many;
  assert.equal(lines.style, "lines");
  assert.equal(lines.items.length, 21);
  assert.deepEqual(lines.items[0], msg("· {}", msg("Unknown permission: {}", "x.0")));
  assert.deepEqual(lines.items.at(-1), msg("{} in total", 25));
  assert.deepEqual(permissionList([]), msg("· Uses none of the player's features"));
});
