/** Tool windows (`plugin.windows`): one Slint window per handle. */
import { basename, extname } from "node:path";
import { failure } from "../errors.js";
import { ViewBase } from "./base.js";
import { kebab } from "./writer.js";
class WindowHandle extends ViewBase {
    #id;
    #file;
    #opened = false;
    #queue = Promise.resolve();
    constructor(scope, file, id) {
        if (scope.runtime.views.has(id))
            throw failure("conflict", `window ${id} already exists; pass a distinct id`);
        super(scope);
        this.#file = file;
        this.#id = id;
        scope.runtime.views.set(id, (callback, args) => this.dispatch(callback, args));
        // The user closed it with the title bar: the host already dropped it.
        this.#closedByUser = scope.runtime.raw("ui.closed", (data) => {
            if (data.view !== this.#id || !this.#opened)
                return;
            this.#opened = false;
            for (const listener of [...this.#closeListeners])
                this.scope.run(listener);
            this.scope.update();
        });
        scope.add(this);
    }
    #closeListeners = new Set();
    #closedByUser;
    onClose(listener) {
        this.#closeListeners.add(listener);
        return this.scope.onDispose(() => {
            this.#closeListeners.delete(listener);
        });
    }
    get opened() {
        return this.#opened;
    }
    live() {
        return this.#opened && !this.closed;
    }
    write(property, write) {
        return this.scope.runtime.host.ui.set(this.#id, property, write);
    }
    on(callback, handler) {
        // The host forwards only callbacks named when the window opens.
        if (this.#opened && !this.handlers.has(kebab(callback)))
            throw new Error(`bind window handler ${callback} before the first show()`);
        return super.on(callback, handler);
    }
    /** Window RPCs run strictly in order; a failure does not block later calls. */
    #serial(task) {
        const next = this.#queue.then(task, task);
        this.#queue = next.catch(() => { });
        return next;
    }
    show() {
        return this.#serial(async () => {
            if (this.closed)
                return;
            const ui = this.scope.runtime.host.ui;
            if (this.#opened)
                return ui.show(this.#id, true);
            const initial = this.wired(this.snapshot(), true);
            await ui.open(this.#id, this.#file, initial, [...this.handlers.keys()]);
            this.#opened = true;
            this.shown(initial);
            this.flush();
        });
    }
    hide() {
        return this.#serial(async () => {
            if (this.#opened)
                await this.scope.runtime.host.ui.show(this.#id, false);
        });
    }
    close() {
        return this.#serial(async () => {
            if (!this.#opened)
                return;
            this.#opened = false;
            await this.scope.runtime.host.ui.close(this.#id);
        });
    }
    async dispose() {
        if (this.closed)
            return;
        this.scope.forget(this);
        await this.close().catch(() => { });
        await this.release();
        await this.#closedByUser.dispose();
        this.#closeListeners.clear();
        this.scope.runtime.views.delete(this.#id);
    }
}
export const windows = (scope) => ({
    create: (file, options) => new WindowHandle(scope, file, options?.id ?? basename(file, extname(file))),
});
