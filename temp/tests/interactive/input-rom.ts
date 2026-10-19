// 实验原生交互测试专用，自制 CC0 ROM；非产品 API，无商业游戏资源。
// 真正的 6502 程序串行读取八个手柄按钮，每个单键选择不同 PPU 背景色。
// 松键后保留240帧，方便原生tap操作取证；若键仍按住则不断刷新TTL，不会自行变黑。
export const releaseFrames = 240;
export const inputKeys = [
  "z",
  "x",
  "Shift",
  "Enter",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
] as const;

export function inputRom(): Buffer {
  const code: number[] = [];
  const labels = new Map<string, number>();
  const fixups: { position: number; label: string; relative: boolean }[] = [];
  const emit = (...bytes: number[]) => code.push(...bytes);
  const label = (name: string) => labels.set(name, 0x8000 + code.length);
  function reference(opcode: number, name: string, relative = false) {
    emit(opcode);
    fixups.push({ position: code.length, label: name, relative });
    if (relative) emit(0);
    else emit(0, 0);
  }
  const write = (address: number, value: number) =>
    emit(0xa9, value, 0x8d, address & 255, address >> 8);
  emit(0x78, 0xd8, 0xa2, 0xff, 0x9a); // SEI / CLD / stack
  write(0x2000, 0);
  write(0x2001, 0);
  write(0x4015, 1);
  write(0x4000, 0xbf); // constant pulse volume; halt length counter during long UI testing
  write(0x4001, 0);
  write(0x4002, 0xfd);
  write(0x4003, 0x08);
  emit(0xa9, 0x0f, 0x85, 1, 0xa9, 0, 0x85, 2); // latched color / TTL
  label("frame");
  emit(0x2c, 0x02, 0x20); // BIT PPUSTATUS, wait for vblank
  reference(0x10, "frame", true);
  write(0x4016, 1);
  write(0x4016, 0);
  emit(0xa9, 0, 0x85, 3); // any button read this frame
  emit(0xa2, 0); // X = controller serial position
  label("read");
  emit(0xad, 0x16, 0x40, 0x29, 1); // LDA $4016 / AND #1
  reference(0xf0, "next", true);
  reference(0xbd, "colors"); // LDA colors,X
  emit(0x85, 1);
  emit(0xa9, releaseFrames, 0x85, 2, 0xa9, 1, 0x85, 3);
  label("next");
  emit(0xe8, 0xe0, 8); // INX / CPX #8
  reference(0xd0, "read", true);
  emit(0xa5, 3);
  reference(0xd0, "render", true);
  emit(0xa5, 2);
  reference(0xf0, "clear", true);
  emit(0xc6, 2); // DEC TTL only when all eight buttons are released
  reference(0xd0, "render", true);
  label("clear");
  emit(0xa9, 0x0f, 0x85, 1);
  label("render");
  write(0x2006, 0x3f);
  write(0x2006, 0);
  emit(0xa5, 1, 0x8d, 0x07, 0x20); // LDA $01 / STA PPUDATA
  write(0x2006, 0);
  write(0x2006, 0);
  write(0x2005, 0);
  write(0x2005, 0);
  write(0x2001, 8);
  reference(0x4c, "frame");
  label("colors");
  emit(0x16, 0x21, 0x2a, 0x12, 0x27, 0x30, 0x19, 0x25);
  for (const fixup of fixups) {
    const target = labels.get(fixup.label);
    if (target === undefined) throw new Error("unresolved fixture ROM label");
    if (fixup.relative) {
      const distance = target - (0x8000 + fixup.position + 1);
      if (distance < -128 || distance > 127)
        throw new Error("fixture branch too far");
      code[fixup.position] = distance & 255;
    } else {
      code[fixup.position] = target & 255;
      code[fixup.position + 1] = target >> 8;
    }
  }
  const rom = Buffer.alloc(16 + 16384 + 8192);
  Buffer.from([78, 69, 83, 26, 1, 1, 0, 0]).copy(rom);
  Buffer.from(code).copy(rom, 16);
  for (const offset of [0x3ffa, 0x3ffc, 0x3ffe])
    rom.writeUInt16LE(0x8000, 16 + offset);
  return rom;
}
