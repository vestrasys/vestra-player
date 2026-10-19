/** A bounded latest-value projection: one drain and one dirty bit, no event backlog. */
export class LatestProjection {
    dirty = false;
    running = null;
    stopped = false;
    publish;
    constructor(publish) {
        this.publish = publish;
    }
    request() {
        if (this.stopped)
            return Promise.resolve();
        this.dirty = true;
        if (!this.running) {
            this.running = this.drain().finally(() => {
                this.running = null;
                // A request can arrive between drain resolution and this finalizer.
                if (this.dirty && !this.stopped)
                    void this.request();
            });
        }
        return this.running;
    }
    async drain() {
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
