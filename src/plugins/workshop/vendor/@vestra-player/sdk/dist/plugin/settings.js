/** Pages of the player's options window (`plugin.settings`, `ui.settings`). */
import { failure, quiet } from "./errors.js";
import { exported, ViewBase } from "./view/base.js";
const CATEGORIES = [
    "general",
    "playback",
    "subtitles",
    "video",
    "audio",
    "input",
    "appearance",
    "plugins",
    "other",
];
const ID = /^[A-Za-z0-9._-]{1,64}$/;
class Page extends ViewBase {
    ready;
    #id;
    #registered = false;
    constructor(scope, file, options) {
        const { id, category, title, order } = options ?? {};
        if (typeof id !== "string" || !ID.test(id))
            throw failure("invalid", "page id: 1–64 letters, digits, . - _");
        if (!CATEGORIES.includes(category))
            throw failure("invalid", `unknown settings category ${category}`);
        if (typeof title !== "string" || !title.trim() || [...title].length > 24)
            throw failure("invalid", "title needs 1–24 characters");
        if (order !== undefined && !Number.isInteger(order))
            throw failure("invalid", "order must be an integer");
        const view = `$settings:${id}`;
        if (scope.runtime.views.has(view))
            throw failure("invalid", `settings page ${id} already exists`);
        super(scope);
        this.#id = id;
        const runtime = scope.runtime;
        runtime.views.set(view, (callback, args) => this.dispatch(callback, args));
        const component = exported(runtime.directory, file, "component");
        const global = exported(runtime.directory, file, "global");
        scope.add(this);
        this.ready = runtime
            .starting((async () => {
            await Promise.resolve(); // bindings declared right after page() join in
            const properties = this.wired(this.snapshot());
            await runtime.host.settings.register({
                id,
                category,
                title,
                ...(order === undefined ? {} : { order }),
                file,
                component,
                global,
                properties,
            });
            this.#registered = true;
            this.shown(properties);
            this.flush();
        })(), (error) => scope.report(error))
            .then(() => { });
        quiet(this.ready);
    }
    live() {
        return this.#registered && !this.closed;
    }
    write(property, write) {
        return this.scope.runtime.host.settings.set(this.#id, property, write);
    }
    async dispose() {
        if (this.closed)
            return;
        this.scope.forget(this);
        if (this.#registered)
            await this.scope.runtime.host.settings
                .unregister(this.#id)
                .catch(() => { });
        this.#registered = false;
        await this.release();
        this.scope.runtime.views.delete(`$settings:${this.#id}`);
    }
}
export const settings = (scope) => ({
    page: (file, options) => new Page(scope, file, options),
    async show(pageId) {
        if (pageId !== undefined &&
            (typeof pageId !== "string" || !ID.test(pageId)))
            throw failure("invalid", "not a settings page of this plugin");
        await scope.runtime.host.settings.show(pageId);
    },
});
