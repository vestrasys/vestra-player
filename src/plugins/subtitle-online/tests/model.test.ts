import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { keywords, movieHash, urlTitle } from "../media.ts";
import { DEFAULTS, parseLanguages, parseSettings, rank } from "../model.ts";
import type { Result } from "../services.ts";

test("moviehash: size plus head and tail as 64-bit little-endian sums", async () => {
  const folder = await mkdtemp(join(tmpdir(), "vestra-hash-"));
  try {
    await writeFile(join(folder, "zero.bin"), Buffer.alloc(131072));
    assert.deepEqual(await movieHash(join(folder, "zero.bin")), { hash: "0000000000020000", size: 131072 });
    const ones = Buffer.alloc(131072);
    ones.writeBigUInt64LE(1n, 0);
    ones.writeBigUInt64LE(0xffffffffffffffffn, 131064);
    await writeFile(join(folder, "wrap.bin"), ones);
    assert.equal((await movieHash(join(folder, "wrap.bin")))!.hash, "0000000000020000", "sums wrap modulo 2^64");
    await writeFile(join(folder, "small.bin"), Buffer.alloc(1000));
    assert.equal(await movieHash(join(folder, "small.bin")), null);
    assert.equal(await movieHash(join(folder, "missing.bin")), null);
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
});

test("keywords drop the extension, release tags and the technical tail", () => {
  assert.equal(keywords("The.Movie.2019.1080p.BluRay.x264-GRP.mkv"), "The Movie 2019");
  assert.equal(keywords("[Group] Show_Name_S01E02_720p.mp4"), "Show Name S01E02");
  assert.equal(keywords("电影 名字.mkv"), "电影 名字");
  assert.equal(urlTitle("https://example.com/videos/Some.Film.2020.WEB-DL.mp4?x=1"), "Some Film 2020");
});

test("settings read leniently; languages are 1–5 codes", () => {
  assert.deepEqual(parseSettings(null), DEFAULTS);
  const read = parseSettings({ services: [{ id: "assrt", enabled: true }, { id: "bogus" }], languages: ["ja"], autoSearch: "always", saveLocation: "data" });
  assert.deepEqual(read.services, [{ id: "assrt", enabled: true }, { id: "opensubtitles", enabled: true }]);
  assert.deepEqual(read.languages, ["ja"]);
  assert.equal(read.autoSearch, "off");
  assert.equal(read.saveLocation, "data");
  assert.deepEqual(parseLanguages("zh-cn, EN，ja"), ["zh-CN", "en", "ja"]);
  assert.equal(parseLanguages("a,b,c,d,e,f"), null);
  assert.equal(parseLanguages("中文"), null);
  assert.equal(parseLanguages(""), null);
});

const result = (key: string, patch: Partial<Result>): Result => ({
  key, service: "opensubtitles", language: "en", release: key, format: "srt", fps: null, downloads: 0,
  hashMatch: false, machine: false, hearingImpaired: false, url: null, ref: 1, fileName: `${key}.srt`, ...patch,
});
test("ranking: hash, then human before machine, then language order, then downloads; filters", () => {
  const list = [
    result("en-many", { downloads: 900 }),
    result("zh", { language: "zh-CN", downloads: 10 }),
    result("tw", { language: "zh-TW", downloads: 50 }),
    result("hash-en", { hashMatch: true }),
    result("mt-zh", { language: "zh-CN", machine: true, downloads: 9999 }),
    result("sample", { release: "Movie SAMPLE cut", downloads: 5000 }),
  ];
  const settings = { languages: ["zh-CN", "zh-TW", "en"], exclude: "", exactOnly: false };
  assert.deepEqual(rank(list, settings).map((r) => r.key), ["hash-en", "zh", "tw", "sample", "en-many", "mt-zh"]);
  assert.deepEqual(rank(list, { ...settings, exclude: "sample, foo" }).map((r) => r.key).includes("sample"), false);
  assert.deepEqual(rank(list, { ...settings, exactOnly: true }).map((r) => r.key), ["hash-en"]);
  assert.equal(rank([...list, list[0]!], settings).length, list.length, "duplicates from More are dropped");
});
