#!/usr/bin/env node
/**
 * Application icons from src/assets/brand/logo.png (coordinator decision 8:
 * an automatic rounded backplate until there is a designed icon).
 *
 *   macOS  AppIcon.icns: the macOS 11+ grid — an 824 px rounded square
 *          (radius 185) with a dark vertical gradient on a 1024 canvas,
 *          the logo scaled to 60 % and centred. PNG entries 16–1024 px.
 *   Windows vestra.ico: the logo itself (Windows icons have no plate),
 *          PNG entries 16–256 px.
 *
 * Pure Node (zlib): same bytes on every OS, no sips/iconutil/ImageMagick.
 *
 *   node packaging/icon.mjs [--icns <file>] [--ico <file>] [--png <file>]
 *   (--png writes the 1024 px macOS master, for review)
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { deflateSync, inflateSync } from "node:zlib";
import { fail, parseArgs, source } from "./lib.mjs";

const LOGO = join(source, "assets/brand/logo.png");

// ------------------------------------------------------------ PNG codec

const CRC = new Int32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c;
});
const crc32 = (bytes) => {
  let c = -1;
  for (const byte of bytes) c = CRC[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
};

/** 8-bit RGBA (or RGB), non-interlaced → { width, height, data: Float32 premultiplied 0–1 }. */
export function decodePng(bytes) {
  if (bytes.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG");
  let offset = 8;
  let header;
  const idat = [];
  while (offset < bytes.length) {
    const length = bytes.readUInt32BE(offset);
    const type = bytes.toString("latin1", offset + 4, offset + 8);
    const body = bytes.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR")
      header = {
        width: body.readUInt32BE(0),
        height: body.readUInt32BE(4),
        depth: body[8],
        color: body[9],
        interlace: body[12],
      };
    else if (type === "IDAT") idat.push(body);
    offset += 12 + length;
  }
  const { width, height, depth, color, interlace } = header;
  const channels = { 2: 3, 6: 4 }[color];
  if (depth !== 8 || !channels || interlace)
    throw new Error(
      "only 8-bit non-interlaced RGB or RGBA images are supported",
    );
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const pixels = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? pixels[y * stride + x - channels] : 0;
      const b = y ? pixels[(y - 1) * stride + x] : 0;
      const c =
        y && x >= channels ? pixels[(y - 1) * stride + x - channels] : 0;
      let value = line[x];
      if (filter === 1) value += a;
      else if (filter === 2) value += b;
      else if (filter === 3) value += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const [pa, pb, pc] = [
          Math.abs(p - a),
          Math.abs(p - b),
          Math.abs(p - c),
        ];
        value += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      pixels[y * stride + x] = value & 0xff;
    }
  }
  const data = new Float32Array(width * height * 4);
  for (let i = 0; i < width * height; i++) {
    const alpha = channels === 4 ? pixels[i * 4 + 3] / 255 : 1;
    for (let k = 0; k < 3; k++)
      data[i * 4 + k] = (pixels[i * channels + k] / 255) * alpha;
    data[i * 4 + 3] = alpha;
  }
  return { width, height, data };
}

export function encodePng({ width, height, data }) {
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0;
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      const alpha = data[i + 3];
      const o = y * (width * 4 + 1) + 1 + x * 4;
      for (let k = 0; k < 3; k++)
        raw[o + k] =
          alpha > 0 ? Math.round(Math.min(1, data[i + k] / alpha) * 255) : 0;
      raw[o + 3] = Math.round(Math.min(1, alpha) * 255);
    }
  }
  const chunk = (type, body) => {
    const head = Buffer.alloc(8);
    head.writeUInt32BE(body.length, 0);
    head.write(type, 4, "latin1");
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), body])), 0);
    return Buffer.concat([head, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ------------------------------------------------------------ drawing

/** Area-average resampling (premultiplied), separable. */
export function resize(image, width, height) {
  const pass = (src, sw, sh, dw, horizontal) => {
    const [ow, oh] = horizontal ? [dw, sh] : [sw, dw];
    const out = new Float32Array(ow * oh * 4);
    const length = horizontal ? sw : sh;
    const scale = length / dw;
    for (let o = 0; o < dw; o++) {
      const start = o * scale;
      const end = start + scale;
      for (
        let s = Math.floor(start);
        s < Math.min(length, Math.ceil(end));
        s++
      ) {
        const weight = (Math.min(end, s + 1) - Math.max(start, s)) / scale;
        if (weight <= 0) continue;
        const lines = horizontal ? sh : sw;
        for (let line = 0; line < lines; line++) {
          const si = horizontal ? (line * sw + s) * 4 : (s * sw + line) * 4;
          const oi = horizontal ? (line * ow + o) * 4 : (o * ow + line) * 4;
          for (let k = 0; k < 4; k++) out[oi + k] += src[si + k] * weight;
        }
      }
    }
    return out;
  };
  const wide = pass(image.data, image.width, image.height, width, true);
  return {
    width,
    height,
    data: pass(wide, width, image.height, height, false),
  };
}

/** Source-over of `top` onto `bottom` at (x, y). */
function composite(bottom, top, left, upper) {
  for (let y = 0; y < top.height; y++)
    for (let x = 0; x < top.width; x++) {
      const bx = x + left;
      const by = y + upper;
      if (bx < 0 || by < 0 || bx >= bottom.width || by >= bottom.height)
        continue;
      const t = (y * top.width + x) * 4;
      const b = (by * bottom.width + bx) * 4;
      const keep = 1 - top.data[t + 3];
      for (let k = 0; k < 4; k++)
        bottom.data[b + k] = top.data[t + k] + bottom.data[b + k] * keep;
    }
}

/** 1024 canvas with the macOS rounded plate and the logo. */
export function macosMaster(logo) {
  const size = 1024;
  const inset = 100;
  const plate = 824;
  const radius = 185;
  const data = new Float32Array(size * size * 4);
  const [top, bottom] = [
    [0.17, 0.17, 0.21],
    [0.06, 0.06, 0.08],
  ];
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      // Signed distance to the rounded square (negative inside).
      const px = Math.abs(x + 0.5 - size / 2) - (plate / 2 - radius);
      const py = Math.abs(y + 0.5 - size / 2) - (plate / 2 - radius);
      const outside = Math.hypot(Math.max(px, 0), Math.max(py, 0));
      const distance = outside + Math.min(Math.max(px, py), 0) - radius;
      const coverage = Math.min(1, Math.max(0, 0.5 - distance));
      if (!coverage) continue;
      const t = Math.min(1, Math.max(0, (y - inset) / plate));
      const i = (y * size + x) * 4;
      for (let k = 0; k < 3; k++)
        data[i + k] = (top[k] + (bottom[k] - top[k]) * t) * coverage;
      data[i + 3] = coverage;
    }
  const canvas = { width: size, height: size, data };
  const scaled = resize(logo, 614, 614);
  composite(canvas, scaled, (size - 614) / 2, (size - 614) / 2);
  return canvas;
}

// ------------------------------------------------------------ containers

/** .icns with PNG entries (macOS 10.7+). */
export function icns(master) {
  const entries = [
    ["icp4", 16],
    ["icp5", 32],
    ["icp6", 64],
    ["ic07", 128],
    ["ic08", 256],
    ["ic09", 512],
    ["ic10", 1024],
    ["ic11", 32],
    ["ic12", 64],
    ["ic13", 256],
    ["ic14", 512],
  ];
  const cache = new Map();
  const png = (size) => {
    if (!cache.has(size))
      cache.set(
        size,
        size === master.width
          ? encodePng(master)
          : encodePng(resize(master, size, size)),
      );
    return cache.get(size);
  };
  const blocks = entries.map(([type, size]) => {
    const body = png(size);
    const head = Buffer.alloc(8);
    head.write(type, 0, "latin1");
    head.writeUInt32BE(body.length + 8, 4);
    return Buffer.concat([head, body]);
  });
  const head = Buffer.alloc(8);
  head.write("icns", 0, "latin1");
  head.writeUInt32BE(
    8 + blocks.reduce((sum, block) => sum + block.length, 0),
    4,
  );
  return Buffer.concat([head, ...blocks]);
}

/** .ico with PNG entries (Windows Vista+). */
export function ico(master) {
  const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
  const images = sizes.map((size) => encodePng(resize(master, size, size)));
  const header = Buffer.alloc(6 + 16 * sizes.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, index) => {
    const entry = 6 + index * 16;
    header[entry] = size >= 256 ? 0 : size;
    header[entry + 1] = size >= 256 ? 0 : size;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(images[index].length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += images[index].length;
  });
  return Buffer.concat([header, ...images]);
}

export async function writeIcons({
  icns: icnsPath,
  ico: icoPath,
  png: pngPath,
}) {
  const logo = decodePng(await readFile(LOGO));
  const write = async (path, bytes) => {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, bytes);
    console.log(`wrote ${path} (${bytes.length} bytes)`);
  };
  if (icnsPath || pngPath) {
    const master = macosMaster(logo);
    if (icnsPath) await write(icnsPath, icns(master));
    if (pngPath) await write(pngPath, encodePng(master));
  }
  if (icoPath) await write(icoPath, ico(logo));
}

if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const options = parseArgs(process.argv.slice(2));
  if (!options.icns && !options.ico && !options.png)
    fail("usage: icon.mjs [--icns <file>] [--ico <file>] [--png <file>]");
  await writeIcons({
    icns: options.icns && resolve(options.icns),
    ico: options.ico && resolve(options.ico),
    png: options.png && resolve(options.png),
  });
}
