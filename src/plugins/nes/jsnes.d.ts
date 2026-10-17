// 实验局部声明：JSNES 2.1.0 的原始声明相对导出不兼容 NodeNext；仅描述所用公开接口。
declare module "jsnes" {
  export interface EmulatorData {
    cpu: object;
    mmap: object;
    ppu: object;
    papu: object;
  }
  export class NES {
    constructor(options: {
      sampleRate: number;
      emulateSound: boolean;
      onFrame: (buffer: Uint32Array) => void;
      onAudioSample: (left: number, right: number) => void;
    });
    loadROM(rom: Buffer): void;
    frame(): void;
    buttonDown(controller: 1 | 2, button: number): void;
    buttonUp(controller: 1 | 2, button: number): void;
    toJSON(): EmulatorData;
    fromJSON(state: EmulatorData): void;
  }
  export class Controller {
    static readonly BUTTON_A: number;
    static readonly BUTTON_B: number;
    static readonly BUTTON_SELECT: number;
    static readonly BUTTON_START: number;
    static readonly BUTTON_UP: number;
    static readonly BUTTON_DOWN: number;
    static readonly BUTTON_LEFT: number;
    static readonly BUTTON_RIGHT: number;
  }
}
