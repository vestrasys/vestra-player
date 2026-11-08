import type { Runtime } from "../runtime.ts";
import type { ScopeImpl } from "../scope.ts";
import type { BackdropSlot, PanelSlot, SidebarSlot, SlotPanel, SlotState } from "../types.ts";
/** The positions `layout.state` reports and `layout.set` changes. */
export type LayoutName = "bottom" | "sidebar" | "adjust";
export type Layout = {
    bottom: SlotState | null;
    sidebar: SlotState | null;
    /** The adjust column. */
    adjust: SlotState | null;
    /** `PanelSlot.panels`. */
    tabs: {
        bottom: readonly SlotPanel[] | null;
        sidebar: readonly SlotPanel[] | null;
        adjust: readonly SlotPanel[] | null;
    };
    backdrop: {
        readonly effective: boolean;
    } | null;
};
/**
 * Both positions, one per plugin: read with `layout.state` after every
 * `window.state` event (one read in flight; events during it cause one more)
 * and after this plugin's own changes. Started on first use.
 */
export declare class LayoutService {
    #private;
    state: Layout;
    constructor(runtime: Runtime);
    /** Read again, but only once something in this plugin uses the state. */
    refresh(): Promise<void>;
    start(): void;
    watch(listener: (layout: Layout) => void): {
        dispose: () => Promise<void>;
    };
    read(): Promise<void>;
    set(slot: LayoutName, change: {
        visible?: boolean;
        detached?: boolean;
        active?: string;
        reveal?: boolean;
    }): Promise<void>;
}
export declare const layoutOf: (runtime: Runtime) => LayoutService;
export declare function slot(scope: ScopeImpl, name: "bottom" | "adjust"): PanelSlot;
export declare function slot(scope: ScopeImpl, name: "sidebar"): SidebarSlot;
export declare function backdrop(scope: ScopeImpl): BackdropSlot;
