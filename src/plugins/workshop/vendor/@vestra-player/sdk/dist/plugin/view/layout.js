/**
 * Both positions, one per plugin: read with `layout.state` after every
 * `window.state` event (one read in flight; events during it cause one more)
 * and after this plugin's own changes. Started on first use.
 */
export class LayoutService {
    #runtime;
    state = {
        bottom: null,
        sidebar: null,
        adjust: null,
        tabs: { bottom: null, sidebar: null, adjust: null },
        backdrop: null,
    };
    #listeners = new Set();
    #started = false;
    #reading = false;
    #again = false;
    constructor(runtime) {
        this.#runtime = runtime;
    }
    /** Read again, but only once something in this plugin uses the state. */
    async refresh() {
        if (this.#started)
            await this.read();
    }
    start() {
        if (this.#started)
            return;
        this.#started = true;
        // A mirror: panels listen through `watch`.
        this.#runtime.on("window.state", () => void this.read(), "events", true);
        void this.read();
    }
    watch(listener) {
        this.start();
        this.#listeners.add(listener);
        const held = this.#runtime.hold("window.state");
        return {
            dispose: async () => {
                this.#listeners.delete(listener);
                await held.dispose();
            },
        };
    }
    async read() {
        if (this.#reading) {
            this.#again = true;
            return;
        }
        this.#reading = true;
        try {
            do {
                this.#again = false;
                // "pending" until the host confirmed the layout: keep the last value.
                const raw = await this.#runtime.host.layout.state().catch(() => null);
                if (!raw)
                    continue;
                const view = (slot) => slot && {
                    preferred: slot.preferred,
                    effective: slot.effective,
                    reason: slot.reason,
                    detached: slot.detached,
                };
                const tabs = (slot) => slot?.panels
                    ? Object.freeze(slot.panels.map((panel) => Object.freeze({
                        plugin: panel.plugin,
                        title: panel.title,
                        active: panel.owner === slot.owner,
                    })))
                    : null;
                const adjust = raw.adjust ?? null;
                const next = {
                    bottom: view(raw.bottom),
                    sidebar: view(raw.sidebar),
                    adjust: view(adjust),
                    tabs: {
                        bottom: tabs(raw.bottom),
                        sidebar: tabs(raw.sidebar),
                        adjust: tabs(adjust),
                    },
                    backdrop: raw.backdrop
                        ? Object.freeze({ effective: raw.backdrop.effective === true })
                        : null,
                };
                if (JSON.stringify(next) === JSON.stringify(this.state))
                    continue;
                this.state = next;
                for (const listener of [...this.#listeners])
                    listener(next);
                this.#runtime.invalidate();
            } while (this.#again);
        }
        finally {
            this.#reading = false;
        }
    }
    async set(slot, change) {
        this.start();
        await this.#runtime.host.layout.set({ slot, ...change });
        await this.read();
    }
}
export const layoutOf = (runtime) => runtime.service("layout", () => new LayoutService(runtime));
export function slot(scope, name) {
    const service = () => layoutOf(scope.runtime);
    return {
        get state() {
            service().start();
            return service().state[name];
        },
        get panels() {
            service().start();
            return service().state.tabs[name];
        },
        onChange(listener) {
            const seen = (layout) => JSON.stringify([layout[name], layout.tabs[name]]);
            let last = seen(service().state);
            const watch = service().watch((layout) => {
                const next = seen(layout);
                if (next === last)
                    return;
                last = next;
                scope.run(listener, layout[name]);
            });
            return scope.add({
                dispose: async () => {
                    scope.forget(watch);
                    await watch.dispose();
                },
            });
        },
        setVisible: (visible) => service().set(name, { visible }),
        async toggle() {
            await service().read();
            const current = service().state[name];
            // A switch (see Panel.toggle): nothing pops out in fullscreen.
            await service().set(name, {
                visible: !(current?.preferred ?? true),
                reveal: false,
            });
        },
        setDetached: (detached) => service().set(name, { detached }),
        activate: (plugin) => service().set(name, { active: plugin }),
    };
}
export function backdrop(scope) {
    const service = () => layoutOf(scope.runtime);
    return {
        get state() {
            service().start();
            return service().state.backdrop;
        },
        onChange(listener) {
            let last = JSON.stringify(service().state.backdrop);
            const watch = service().watch((layout) => {
                const next = JSON.stringify(layout.backdrop);
                if (next === last)
                    return;
                last = next;
                scope.run(listener, layout.backdrop);
            });
            return scope.add({
                dispose: async () => {
                    scope.forget(watch);
                    await watch.dispose();
                },
            });
        },
    };
}
