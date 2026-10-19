// 实验测试专用、自制 CC0 ROM；无商业游戏数据。6502 程序读取 A 键改变背景并驱动 APU。
export function testRom(): Buffer {
  const code: number[] = [];
  const labels = new Map<string, number>();
  const fixups: { position: number; label: string; relative: boolean }[] = [];
  const emit = (...bytes: number[]) => code.push(...bytes);
  const label = (name: string) => labels.set(name, 0x8000 + code.length);
  const jump = (opcode: number, name: string) => {
    emit(opcode);
    fixups.push({ position: code.length, label: name, relative: false });
    emit(0, 0);
  };
  const branch = (opcode: number, name: string) => {
    emit(opcode);
    fixups.push({ position: code.length, label: name, relative: true });
    emit(0);
  };
  const write = (address: number, value: number) =>
    emit(0xa9, value, 0x8d, address & 255, address >> 8);
  emit(0x78, 0xd8, 0xa2, 0xff, 0x9a); // SEI CLD LDX #$ff TXS
  write(0x2000, 0);
  write(0x2001, 0);
  write(0x4015, 1);
  write(0x4000, 0x9f);
  write(0x4001, 0);
  write(0x4002, 0xfd);
  write(0x4003, 0x08);
  label("loop");
  label("wait");
  emit(0x2c, 0x02, 0x20);
  branch(0x10, "wait"); // BIT PPUSTATUS / BPL
  write(0x4016, 1);
  write(0x4016, 0);
  emit(0xad, 0x16, 0x40, 0x29, 1);
  branch(0xf0, "released");
  emit(0xa9, 0x21);
  jump(0x4c, "color");
  label("released");
  emit(0xa9, 0x16);
  label("color");
  emit(0x48);
  write(0x2006, 0x3f);
  write(0x2006, 0);
  emit(0x68, 0x8d, 0x07, 0x20);
  write(0x2006, 0);
  write(0x2006, 0);
  write(0x2005, 0);
  write(0x2005, 0);
  write(0x2001, 0x08);
  jump(0x4c, "loop");
  for (const fix of fixups) {
    const address = labels.get(fix.label);
    if (address === undefined) throw new Error("ROM assembler label missing");
    if (fix.relative)
      code[fix.position] = (address - (0x8000 + fix.position + 1)) & 255;
    else {
      code[fix.position] = address & 255;
      code[fix.position + 1] = address >> 8;
    }
  }
  const rom = Buffer.alloc(16 + 16384 + 8192);
  Buffer.from([78, 69, 83, 26, 1, 1, 0, 0]).copy(rom);
  Buffer.from(code).copy(rom, 16);
  for (const offset of [0x3ffa, 0x3ffc, 0x3ffe])
    rom.writeUInt16LE(0x8000, 16 + offset);
  return rom;
}
