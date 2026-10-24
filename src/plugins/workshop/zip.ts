// A zip file in pure Node (deflate, UTF-8 names): the same input always
// gives the same bytes (entries in the given order, a fixed timestamp).
import { crc32, deflateRawSync } from "node:zlib";

export interface ZipEntry {
  /** "/"-separated path inside the archive. */
  name: string;
  data: Buffer;
}

/** 1980-01-01 00:00, the earliest DOS date. */
const DOS_TIME = 0;
const DOS_DATE = (0 << 9) | (1 << 5) | 1;

export function zip(entries: ZipEntry[]): Buffer {
  const local: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const { name, data } of entries) {
    if (!name || name.startsWith("/") || name.split("/").includes("..")) throw new Error(`bad zip entry name: ${name}`);
    const file = Buffer.from(name, "utf8");
    const compressed = deflateRawSync(data, { level: 9 });
    const checksum = crc32(data);
    const header = Buffer.alloc(30);
    header.writeUInt32LE(0x04034b50, 0);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(0x0800, 6); // UTF-8 names
    header.writeUInt16LE(8, 8); // deflate
    header.writeUInt16LE(DOS_TIME, 10);
    header.writeUInt16LE(DOS_DATE, 12);
    header.writeUInt32LE(checksum, 14);
    header.writeUInt32LE(compressed.length, 18);
    header.writeUInt32LE(data.length, 22);
    header.writeUInt16LE(file.length, 26);
    local.push(header, file, compressed);
    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(0x0314, 4); // made by: Unix, 2.0 (permissions below)
    entry.writeUInt16LE(20, 6);
    entry.writeUInt16LE(0x0800, 8);
    entry.writeUInt16LE(8, 10);
    entry.writeUInt16LE(DOS_TIME, 12);
    entry.writeUInt16LE(DOS_DATE, 14);
    entry.writeUInt32LE(checksum, 16);
    entry.writeUInt32LE(compressed.length, 20);
    entry.writeUInt32LE(data.length, 24);
    entry.writeUInt16LE(file.length, 28);
    entry.writeUInt32LE((0o100644 << 16) >>> 0, 38); // a regular file, rw-r--r--
    entry.writeUInt32LE(offset, 42);
    central.push(entry, file);
    offset += header.length + file.length + compressed.length;
  }
  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...local, directory, end]);
}
