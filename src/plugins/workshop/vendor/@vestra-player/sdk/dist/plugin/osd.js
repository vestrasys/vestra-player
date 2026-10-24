/** On-screen display: a transient line and a persistent information block. */
import { Latest } from "./runtime.js";
import { failure } from "./runtime.js";
/** Copy and check template arguments: an array or object of ≤ 16 strings and numbers. */
export function templateArgs(args) {
    if (args === undefined || args === null)
        return undefined;
    const values = Array.isArray(args)
        ? args
        : typeof args === "object"
            ? Object.values(args)
            : null;
    if (!values ||
        values.length > 16 ||
        values.some((value) => !((typeof value === "number" && Number.isFinite(value)) ||
            typeof value === "string")))
        throw failure("invalid", "args is an array or object of at most 16 strings and finite numbers");
    return Array.isArray(args)
        ? [...args]
        : { ...args };
}
class PanelHandle {
    #scope;
    #pump;
    /** The last value sent, as JSON (`null` text: hidden). */
    #last = JSON.stringify({ text: null });
    #disposed = false;
    constructor(scope) {
        if (scope.runtime.services.has("osd.panel"))
            throw new Error("a plugin can have only one osd panel");
        scope.runtime.services.set("osd.panel", this);
        this.#scope = scope;
        this.#pump = new Latest(({ text, args }) => scope.runtime.host.osd.panel(text, args));
        scope.add(this);
    }
    set(text, args) {
        if (this.#disposed)
            return;
        let value;
        try {
            value = {
                text,
                ...(text === null || args === undefined
                    ? {}
                    : { args: templateArgs(args) }),
            };
        }
        catch (error) {
            this.#scope.report(error);
            return;
        }
        const key = JSON.stringify(value);
        if (key === this.#last)
            return;
        this.#last = key;
        this.#pump.push(value).catch((error) => this.#scope.report(error));
    }
    async dispose() {
        if (this.#disposed)
            return;
        this.#disposed = true;
        this.#scope.forget(this);
        this.#scope.runtime.services.delete("osd.panel");
        if (this.#last !== JSON.stringify({ text: null }))
            await this.#pump.push({ text: null }).catch(() => { });
        this.#pump.stop();
    }
}
export const osd = (scope) => ({
    show: async (text, args) => scope.runtime.host.osd.show(text, templateArgs(args)),
    panel: () => new PanelHandle(scope),
});
