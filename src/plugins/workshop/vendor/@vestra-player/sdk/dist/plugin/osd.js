/** On-screen display: a transient line and a persistent information block. */
import { Latest } from "./runtime.js";
class PanelHandle {
    #scope;
    #pump;
    #last = null;
    #disposed = false;
    constructor(scope) {
        if (scope.runtime.services.has("osd.panel"))
            throw new Error("a plugin can have only one osd panel");
        scope.runtime.services.set("osd.panel", this);
        this.#scope = scope;
        this.#pump = new Latest((text) => scope.runtime.host.osd.panel(text));
        scope.add(this);
    }
    set(text) {
        if (this.#disposed || text === this.#last)
            return;
        this.#last = text;
        this.#pump.push(text).catch((error) => this.#scope.report(error));
    }
    async dispose() {
        if (this.#disposed)
            return;
        this.#disposed = true;
        this.#scope.forget(this);
        this.#scope.runtime.services.delete("osd.panel");
        if (this.#last !== null)
            await this.#pump.push(null).catch(() => { });
        this.#pump.stop();
    }
}
export const osd = (scope) => ({
    show: (text) => scope.runtime.host.osd.show(text),
    panel: () => new PanelHandle(scope),
});
