import assert from "node:assert/strict";
import test from "node:test";
import { convert, cuesOf, decode, parseAss, parseSrt } from "../formats.ts";
import { clock, dialogueTarget, parseStyle } from "../model.ts";

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);
const options = { fallback: "gb18030" as const, fps: null };

test("encodings: BOMs, strict UTF-8, then the fallback (GB18030 by default)", () => {
  const srt = "1\n00:00:01,000 --> 00:00:02,000\n你好，世界\n";
  const gb = new Uint8Array(Buffer.from([0x31, 0x0a]));
  assert.equal(decode(gb, "gb18030").utf8, true);
  // "你好" in GB18030
  const gbk = Buffer.concat([Buffer.from("1\n00:00:01,000 --> 00:00:02,000\n"), Buffer.from([0xc4, 0xe3, 0xba, 0xc3]), Buffer.from("\n")]);
  const converted = convert(gbk, options);
  assert.equal(converted.kind, "SRT");
  assert.match(text(converted.tracks[0]!.bytes), /你好/);
  assert.equal(converted.tracks[0]!.format, "srt");
  const big5 = convert(Buffer.concat([Buffer.from("1\n00:00:01,000 --> 00:00:02,000\n"), Buffer.from([0xa7, 0x41, 0xa6, 0x6e])]), {
    ...options, fallback: "big5",
  });
  assert.match(text(big5.tracks[0]!.bytes), /你好/);
  const utf16 = Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(srt, "utf16le")]);
  assert.equal(text(convert(utf16, options).tracks[0]!.bytes), srt);
  const utf8 = Buffer.from(srt);
  assert.equal(convert(utf8, options).tracks[0]!.bytes.equals(utf8), true, "UTF-8 SRT is uploaded byte for byte");
});

test("ASS/SSA pass through without [Fonts] and [Graphics]; UTF-16LE is re-encoded", () => {
  const ass = [
    "[Script Info]", "ScriptType: v4.00+", "", "[V4+ Styles]", "Format: Name, Fontname", "Style: Default,Arial", "",
    "[Fonts]", "fontname: a.ttf", "M0!!#$%", "", "[Events]",
    "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    "Dialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\pos(10,10)\\fad(200,200)}你好,世界\\N第二行", "",
    "[Graphics]", "logo.png", "",
  ].join("\r\n");
  const result = convert(Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(ass, "utf16le")]), options);
  assert.equal(result.kind, "ASS");
  assert.equal(result.tracks[0]!.format, "ass");
  const out = text(result.tracks[0]!.bytes);
  assert.doesNotMatch(out, /\[Fonts\]|\[Graphics\]|a\.ttf|logo\.png/);
  assert.match(out, /\\pos\(10,10\)/, "override tags are kept for the renderer");
  assert.deepEqual(result.notes, ["已忽略字幕内嵌字体"]);
  assert.deepEqual(parseAss(out), [{ start: 1.5, end: 3, text: "你好,世界\n第二行" }]);
  assert.equal(convert(Buffer.from(ass.replace("v4.00+", "v4.00")), options).kind, "SSA");
});

test("WebVTT becomes SRT: settings, NOTE/STYLE blocks and <c>/<v> tags dropped", () => {
  const vtt = [
    "WEBVTT - demo", "", "STYLE", "::cue { color: red }", "", "NOTE a comment", "",
    "intro", "00:01.000 --> 00:02.500 align:start", "<v Bob>Hello <c.yellow>there</c></v> &amp; <b>you</b>", "",
    "01:00:00.000 --> 01:00:01.000", "Line 1", "Line <00:00:00.500>2", "",
  ].join("\n");
  const result = convert(Buffer.from(vtt), options);
  assert.equal(result.kind, "WebVTT");
  assert.equal(
    text(result.tracks[0]!.bytes),
    "1\n00:00:01,000 --> 00:00:02,500\nHello there & <b>you</b>\n\n2\n01:00:00,000 --> 01:00:01,000\nLine 1\nLine 2\n",
  );
});

test("SAMI with two classes gives one SRT track per language", () => {
  const smi = `<SAMI><HEAD><STYLE><!--
.ENCC { Name: English; lang: en-US; }
.KRCC { Name: 中文; lang: zh-CN; }
--></STYLE></HEAD><BODY>
<SYNC Start=1000><P Class=ENCC>Hello<br>world
<SYNC Start=1000><P Class=KRCC>你好
<SYNC Start=3000><P Class=ENCC>&nbsp;
<SYNC Start=3500><P Class=KRCC>&nbsp;
</BODY></SAMI>`;
  const result = convert(Buffer.from(smi), options);
  assert.equal(result.kind, "SAMI");
  assert.deepEqual(result.tracks.map((t) => t.language), ["English", "中文"]);
  assert.equal(text(result.tracks[0]!.bytes), "1\n00:00:01,000 --> 00:00:03,000\nHello\nworld\n");
  assert.equal(text(result.tracks[1]!.bytes), "1\n00:00:01,000 --> 00:00:03,500\n你好\n");
});

test("MicroDVD uses the video frame rate, then the declared one, then 23.976 with a note", () => {
  const sub = "{1}{1}25\n{25}{50}Hello|world\n{100}{125}{y:i}Bye\n";
  assert.equal(text(convert(Buffer.from(sub), options).tracks[0]!.bytes).split("\n")[1], "00:00:01,000 --> 00:00:02,000");
  const video = convert(Buffer.from(sub), { ...options, fps: 50 });
  assert.equal(text(video.tracks[0]!.bytes).split("\n")[1], "00:00:00,500 --> 00:00:01,000");
  const plain = convert(Buffer.from("{24}{48}Hi\n"), options);
  assert.deepEqual(plain.notes, ["视频帧率未知，已按 23.976 换算 MicroDVD 字幕"]);
  assert.match(text(plain.tracks[0]!.bytes), /Hi/);
});

test("graphic, unknown and empty files are refused with a reason", () => {
  assert.throws(() => convert(Buffer.from("# VobSub index file, v7\nsize: 720x480\n"), options), /暂不支持图形字幕文件/);
  assert.throws(() => convert(Buffer.from([0x00, 0x00, 0x01, 0xba, 0x44]), options), /暂不支持图形字幕文件/);
  assert.throws(() => convert(Buffer.from("PG\u0000\u0000"), options), /暂不支持图形字幕文件/);
  assert.throws(() => convert(Buffer.from("00:01 hello\n00:02 world\n"), options), /无法识别的字幕格式/);
  assert.throws(() => convert(Buffer.from("WEBVTT\n\n"), options), /没有字幕/);
});

test("cues for B11: SRT and ASS times; dialogue targets follow the sync offset", () => {
  const srt = "1\n00:00:10,000 --> 00:00:12,000\nA\n\n2\n00:00:20,000 --> 00:00:22,000\nB\n\n3\n00:00:30,000 --> 00:00:32,000\n<i>C</i>\n";
  const cues = parseSrt(srt);
  assert.deepEqual(cues.map((c) => [c.start, c.end, c.text]), [[10, 12, "A"], [20, 22, "B"], [30, 32, "C"]]);
  assert.deepEqual(cuesOf("srt", Buffer.from(srt)), cues);
  assert.equal(dialogueTarget(cues, 25, 0, "next"), 30);
  assert.equal(dialogueTarget(cues, 25, 0, "previous"), 20);
  assert.equal(dialogueTarget(cues, 20.3, 0, "previous"), 10, "within 0.5 s of a start goes one further back");
  assert.equal(dialogueTarget(cues, 25, 1000, "next"), 31, "+1000 ms shows cues one second later");
  assert.equal(dialogueTarget(cues, 21, 0, "current"), 20);
  assert.equal(dialogueTarget(cues, 25, 0, "current"), 20, "between cues: the last one that ended");
  assert.equal(dialogueTarget(cues, 31, 0, "next"), null);
  assert.equal(dialogueTarget(cues, 10.2, 0, "previous"), null);
  assert.equal(clock(3723.45), "01:02:03.450");
});

test("style.json is read leniently: out-of-range fields take their defaults", () => {
  const base = { fontId: "sans", fontSize: 36, color: "white", outline: 2, marginPercent: 5, bold: false, shadow: 0, box: null, assOverride: "keep" } as const;
  assert.deepEqual(parseStyle(null, base), { style: null, shadowLast: 2, boxOpacity: 50 });
  const read = parseStyle({ style: { fontId: "", fontSize: 99, color: "red", bold: true, shadow: 3, box: 70, assOverride: "force" }, shadowLast: 9, boxOpacity: 45 }, base);
  assert.deepEqual(read, {
    style: { fontId: "sans", fontSize: 36, color: "white", outline: 2, marginPercent: 5, bold: true, shadow: 3, box: 70, assOverride: "force" },
    shadowLast: 2,
    boxOpacity: 45,
  });
});
