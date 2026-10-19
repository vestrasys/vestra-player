/** A bounded latest-value projection: one drain and one dirty bit, no event backlog. */
export class LatestProjection {
  private dirty = false;
  private running: Promise<void> | null = null;
  private stopped = false;
  private readonly publish: () => Promise<void>;
  constructor(publish: () => Promise<void>) {
    this.publish = publish;
  }
  request(): Promise<void> {
    if (this.stopped) return Promise.resolve();
    this.dirty = true;
    if (!this.running) {
      this.running = this.drain().finally(() => {
        this.running = null;
        // A request can arrive between drain resolution and this finalizer.
        if (this.dirty && !this.stopped) void this.request();
      });
    }
    return this.running;
  }
  private async drain() {
    while (this.dirty && !this.stopped) {
      this.dirty = false;
      await this.publish();
    }
  }
  stop() {
    this.stopped = true;
    this.dirty = false;
  }
}
