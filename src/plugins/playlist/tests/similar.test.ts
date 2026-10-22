// IA-PL-01 「同类文件规则」: the 17 cases of doc/product/interaction-spec.md, run
// through `siblings` on real folders, and the step each similar file passes
// (`similarStep`), plus the helpers' edge cases.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { basename, extname, join } from "node:path";
import { tmpdir } from "node:os";
import {
  canHaveSimilar,
  digitDiff,
  loosePrefix,
  mask,
  maskEq,
  SCAN_LIMIT,
  similarKey,
  similarStep,
  siblings,
  type SimilarStep,
} from "../media.ts";

type Case = {
  open: string;
  folder: string[];
  expected: string[];
  mode?: "similar" | "folder";
  /** Candidate → the step that takes it (similar mode). */
  steps?: Record<string, SimilarStep>;
};

const cases: Case[] = [
  {
    open: "Show.S01E02.1080p.mkv",
    folder: [
      "Show.S01E01.1080p.mkv",
      "Show.S01E02.1080p.mkv",
      "Show.S01E10.1080p.mkv",
      "Show.S02E01.1080p.mkv",
      "Show.S01E03.720p.mp4",
      "Show.S01E01.1080p.srt",
      "Other.Movie.2019.mkv",
    ],
    expected: [
      "Show.S01E01.1080p.mkv",
      "Show.S01E02.1080p.mkv",
      "Show.S01E03.720p.mp4",
      "Show.S01E10.1080p.mkv",
      "Show.S02E01.1080p.mkv",
    ],
    steps: {
      "Show.S01E01.1080p.mkv": "S1",
      "Show.S01E03.720p.mp4": "S1",
      "Show.S01E10.1080p.mkv": "S3 equal",
      "Show.S02E01.1080p.mkv": "S3 equal",
    },
  },
  {
    open: "第02集.mp4",
    folder: ["第01集.mp4", "第02集.mp4", "第10集.mp4", "第01集.mkv", "花絮.mp4"],
    expected: ["第01集.mkv", "第01集.mp4", "第02集.mp4", "第10集.mp4"],
    steps: { "第01集.mp4": "S3 equal", "第10集.mp4": "S3 equal", "第01集.mkv": "S3 equal" },
  },
  {
    open: "[字幕组] 进击的巨人 - 02 [1080p].mkv",
    folder: [
      "[字幕组] 进击的巨人 - 01 [1080p].mkv",
      "[字幕组] 进击的巨人 - 02 [1080p].mkv",
      "[字幕组] 进击的巨人 - 13 [1080p].mkv",
      "[字幕组] 鬼灭之刃 - 01 [1080p].mkv",
      "[其他组] 进击的巨人 - 03 [720p].mkv",
    ],
    expected: [
      "[字幕组] 鬼灭之刃 - 01 [1080p].mkv",
      "[字幕组] 进击的巨人 - 01 [1080p].mkv",
      "[字幕组] 进击的巨人 - 02 [1080p].mkv",
      "[字幕组] 进击的巨人 - 13 [1080p].mkv",
    ],
    steps: {
      "[字幕组] 进击的巨人 - 01 [1080p].mkv": "S1",
      "[字幕组] 进击的巨人 - 13 [1080p].mkv": "S3 equal",
      "[字幕组] 鬼灭之刃 - 01 [1080p].mkv": "S3 prefix",
    },
  },
  {
    open: "A02.mkv",
    folder: ["A01.mkv", "A02.mkv", "A03.mkv", "B.mkv", "song.mp3"],
    expected: ["A01.mkv", "A02.mkv", "A03.mkv"],
    steps: { "A01.mkv": "S3 equal", "A03.mkv": "S3 equal" },
  },
  {
    open: "A02.mkv",
    mode: "folder",
    folder: ["A01.mkv", "A02.mkv", "A03.mkv", "B.mkv", "song.mp3", "notes.txt"],
    expected: ["A01.mkv", "A02.mkv", "A03.mkv", "B.mkv"],
  },
  { open: "movie.mkv", folder: ["movie.mkv", "movie2.mkv", "trailer.mkv"], expected: ["movie.mkv"] },
  {
    open: "ep1.mp4",
    folder: ["ep1.mp4", "ep2.mp4", "EP3.MP4", "ep4.avi", "ep10.mp4", "ep5.mp3"],
    expected: ["ep1.mp4", "ep2.mp4", "EP3.MP4", "ep4.avi", "ep10.mp4"],
    steps: { "ep2.mp4": "S3 equal", "EP3.MP4": "S3 equal", "ep4.avi": "S3 equal", "ep10.mp4": "S3 equal" },
  },
  {
    open: "Ep 001.mkv",
    folder: ["Ep 001.mkv", "Ep 002.mkv", "Ep 010.mkv", "Ep 100.mkv", "Ep 1000.mkv"],
    expected: ["Ep 001.mkv", "Ep 002.mkv", "Ep 010.mkv", "Ep 100.mkv", "Ep 1000.mkv"],
    steps: { "Ep 002.mkv": "S1", "Ep 010.mkv": "S3 equal", "Ep 100.mkv": "S3 equal", "Ep 1000.mkv": "S3 equal" },
  },
  {
    open: "01 - Intro.mp3",
    folder: ["01 - Intro.mp3", "02 - Song Two.mp3", "03 - Another.flac", "10 - Outro.mp3", "Interview.mp3", "cover.jpg"],
    expected: ["01 - Intro.mp3", "02 - Song Two.mp3", "03 - Another.flac", "10 - Outro.mp3"],
    steps: { "02 - Song Two.mp3": "S4 audio", "03 - Another.flac": "S4 audio", "10 - Outro.mp3": "S4 audio" },
  },
  {
    open: "Vlog_2023-05-01.mp4",
    folder: ["Vlog_2023-05-01.mp4", "Vlog_2023-05-02.mp4", "Vlog_2024-12-31.mp4", "Holiday_2023-05-01.mp4"],
    expected: ["Vlog_2023-05-01.mp4", "Vlog_2023-05-02.mp4", "Vlog_2024-12-31.mp4"],
    steps: { "Vlog_2023-05-02.mp4": "S1", "Vlog_2024-12-31.mp4": "S1" },
  },
  {
    open: "Lecture 1.mp4",
    folder: ["Lecture 1.mp4", "Lecture 2.mp4", "lecture 3.mp4", "Lecture 12.mp4", "Lab 1.mp4"],
    expected: ["Lecture 1.mp4", "Lecture 2.mp4", "lecture 3.mp4", "Lecture 12.mp4"],
    steps: { "Lecture 2.mp4": "S1", "lecture 3.mp4": "S1", "Lecture 12.mp4": "S1" },
  },
  {
    open: "Friends.S01E01.mkv",
    folder: ["Friends.S01E01.mkv", "Friends.S01E02.mkv", "Friends.Special.mkv", "Friends.S01E01.ass", "Friends.S01E03.zip"],
    expected: ["Friends.S01E01.mkv", "Friends.S01E02.mkv", "Friends.Special.mkv"],
    steps: { "Friends.S01E02.mkv": "S1", "Friends.Special.mkv": "S1" },
  },
  {
    open: "西游记01.rmvb",
    folder: ["西游记01.rmvb", "西游记02.rmvb", "西游记25.rmvb", "红楼梦01.rmvb"],
    expected: ["西游记01.rmvb", "西游记02.rmvb", "西游记25.rmvb"],
    steps: { "西游记02.rmvb": "S3 equal", "西游记25.rmvb": "S3 equal" },
  },
  {
    open: "abc-01.mkv",
    folder: ["abc-01.mkv", "abc-02.mkv", "abd-01.mkv", "xyz-01.mkv"],
    expected: ["abc-01.mkv", "abc-02.mkv", "abd-01.mkv"],
    steps: { "abc-02.mkv": "S1", "abd-01.mkv": "S1" },
  },
  {
    open: "Show.S01E01.mkv",
    folder: ["Show.S01E01.mkv", "Show.S01E02.mkv", "Shower.Cam.mkv", "Showtime.S01E01.mkv"],
    expected: ["Show.S01E01.mkv", "Show.S01E02.mkv", "Shower.Cam.mkv"],
    steps: { "Show.S01E02.mkv": "S1", "Shower.Cam.mkv": "S4 mask" },
  },
  { open: "第０２集.mp4", folder: ["第０１集.mp4", "第０２集.mp4", "第１０集.mp4"], expected: ["第０２集.mp4"] },
  {
    open: "ab1.mkv",
    folder: ["ab1.mkv", "ab2.mkv", "ac1.mkv", "b1.mkv"],
    expected: ["ab1.mkv", "ab2.mkv"],
    steps: { "ab2.mkv": "S3 equal" },
  },
];

const key = (name: string) => similarKey(basename(name, extname(name)));
const audio = (name: string) => /\.(mp3|flac)$/i.test(name);

cases.forEach((c, index) => {
  test(`IA-PL-01 case ${index + 1}: ${c.open}${c.mode === "folder" ? " (全部)" : ""}`, async () => {
    const dir = await realpath(await mkdtemp(join(tmpdir(), "vestra-similar-")));
    try {
      for (const name of c.folder) await writeFile(join(dir, name), "x");
      const { paths, truncated } = await siblings(join(dir, c.open), c.mode ?? "similar");
      assert.deepEqual(
        paths.map((path) => basename(path)),
        c.expected,
      );
      assert.equal(truncated, false);
      if (c.mode === "folder") return;
      // Each similar file passes the step the table names; every other file of the same group passes none.
      for (const name of c.folder) {
        if (name === c.open) continue;
        const step = similarStep(key(c.open), key(name), audio(c.open) && audio(name));
        if (c.expected.includes(name)) assert.equal(step, c.steps?.[name], name);
        else if (
          canHaveSimilar(basename(c.open, extname(c.open))) &&
          /\.(mkv|mp4|avi|rmvb|mp3|flac)$/i.test(name) &&
          audio(name) === audio(c.open)
        )
          assert.equal(step, null, name);
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

test("IA-PL-01: the lengths count the `.tmp` (case 17); short names are only similar when equal without digits", () => {
  assert.equal(similarKey("AB1"), "ab1.tmp");
  assert.equal(similarKey("ÄB1"), "Äb1.tmp", "only A–Z are lowered");
  assert.equal(similarKey("ab1").length, 7); // 7 / 2 = 3 < 5: S1 is not tried
  assert.equal(similarStep(similarKey("ab1"), similarKey("ac1"), false), null);
  assert.equal(similarStep(similarKey("ab1"), similarKey("b1"), false), null);
});

test("IA-PL-01: the precondition needs a space, -, _ or an ASCII digit and two characters", () => {
  for (const stem of ["Ep 1", "a-b", "a_b", "x9", "第02集"]) assert.equal(canHaveSimilar(stem), true, stem);
  for (const stem of ["movie", "第０２集", "第二集", "1"]) assert.equal(canHaveSimilar(stem), false, stem);
});

test("IA-PL-01 helpers: digitDiff, loosePrefix, maskEq and mask", () => {
  assert.equal(digitDiff("20230501_x", "20230502_y", 3), 0);
  assert.equal(digitDiff("20230501", "20230512", 8), 2);
  assert.equal(digitDiff("2023a", "20230", 5), 100);
  assert.equal(digitDiff("１２", "１２", 2), 100, "full-width digits are not digits");
  // A mismatch between two ASCII characters where one is a digit stops the count.
  assert.equal(loosePrefix("ab1cdefgh", "ab2cdefgh"), false);
  // Other mismatches are skipped (2 of 3 compared match, the longer's half is 1).
  assert.equal(loosePrefix("axc", "ayc"), true);
  assert.equal(mask("ep 01.tmp"), "ep?**?tmp");
  // S4: a digit against a non-digit at position n is not similar.
  assert.equal(maskEq("ab*cdef", "abxcdef", 2, true), false);
  assert.equal(maskEq("abcxyz", "abcpqr", 3, true), true);
  assert.equal(maskEq("abc", "abcd", 2, false), false, "not a prefix: the lengths must agree");
  assert.equal(maskEq("ab", "abcd", 5, true), true, "all compared characters equal");
});

test("IA-PL-01: names starting with `.` are skipped, folders too; more than SCAN_LIMIT matches are cut around the opened file", async () => {
  const dir = await realpath(await mkdtemp(join(tmpdir(), "vestra-similar-")));
  try {
    await writeFile(join(dir, "A01.mkv"), "x");
    await writeFile(join(dir, "._A02.mkv"), "x");
    await writeFile(join(dir, "A03.mkv"), "x");
    await mkdir(join(dir, "A04.mkv"));
    const { paths } = await siblings(join(dir, "A01.mkv"), "folder");
    assert.deepEqual(paths.map((path) => basename(path)), ["A01.mkv", "A03.mkv"]);

    const big = join(dir, "big");
    await mkdir(big);
    for (let i = 0; i < SCAN_LIMIT + 10; i++) await writeFile(join(big, `ep${String(i).padStart(5, "0")}.mp4`), "");
    const opened = join(big, "ep05005.mp4");
    const cut = await siblings(opened, "folder");
    assert.equal(cut.truncated, true);
    assert.equal(cut.paths.length, SCAN_LIMIT);
    assert.ok(cut.paths.includes(opened));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
