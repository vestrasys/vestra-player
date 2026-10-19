/** Local drafts and a bounded, latest-only submission queue; never playback facts. */
export class LatestValue<T> {
  private queued: T | undefined;
  private running = false;
  private epoch = 0;
  private readonly submit: (value: T) => Promise<void>;
  private readonly failed: (error: unknown) => void;
  constructor(
    submit: (value: T) => Promise<void>,
    failed: (error: unknown) => void,
  ) {
    this.submit = submit;
    this.failed = failed;
  }
  push(value: T) {
    this.queued = value;
    if (!this.running) void this.drain();
  }
  clear() {
    this.queued = undefined;
    this.epoch++;
  }
  private async drain() {
    this.running = true;
    try {
      while (this.queued !== undefined) {
        const value = this.queued;
        const epoch = this.epoch;
        this.queued = undefined;
        const started = Date.now();
        try {
          await this.submit(value);
        } catch (error) {
          if (epoch === this.epoch) {
            this.queued = undefined;
            this.failed(error);
          }
        }
        const wait = 34 - (Date.now() - started);
        if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
      }
    } finally {
      this.running = false;
    }
  }
}
export function time(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return "--:--";
  const whole = Math.max(0, Math.floor(seconds));
  const s = String(whole % 60).padStart(2, "0");
  const m = String(Math.floor(whole / 60) % 60).padStart(2, "0");
  return whole >= 3600 ? `${Math.floor(whole / 3600)}:${m}:${s}` : `${m}:${s}`;
}
export function parseTime(value: string): number | null {
  if (!/^\d+(?::[0-5]\d){0,2}(?:\.\d{1,3})?$/.test(value.trim())) return null;
  const parts = value.trim().split(":").map(Number);
  const result = parts.reduce((total, part) => total * 60 + part, 0);
  return Number.isFinite(result) ? result : null;
}

export type Stamp = { generation: number; intentVersion: number };
const same = (a: Stamp, b: Stamp) =>
  a.generation === b.generation && a.intentVersion === b.intentVersion;
/** A gesture can advance only via its own operation receipt, never arbitrary shared state. */
export class SeekFence {
  private allowed: Stamp;
  private pending: Stamp | null = null;
  valid = true;
  constructor(stamp: Stamp) {
    this.allowed = { ...stamp };
  }
  begin(): Stamp | null {
    if (!this.valid || this.pending) return null;
    this.pending = {
      ...this.allowed,
      intentVersion: this.allowed.intentVersion + 1,
    };
    return { ...this.allowed };
  }
  observe(stamp: Stamp): boolean {
    if (!this.valid) return false;
    if (
      !same(stamp, this.allowed) &&
      !(this.pending && same(stamp, this.pending))
    )
      this.valid = false;
    return this.valid;
  }
  complete(stamp: Stamp): boolean {
    if (!this.valid || !this.pending || !same(stamp, this.pending)) {
      this.valid = false;
      return false;
    }
    this.allowed = { ...stamp };
    this.pending = null;
    return true;
  }
  cancel() {
    this.valid = false;
  }
}
