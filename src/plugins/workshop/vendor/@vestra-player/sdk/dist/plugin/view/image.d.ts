import type { Json } from "../../wire.ts";
import type { ScopeImpl } from "../scope.ts";
import type { ImageOptions, ImageRef, Ui } from "../types.ts";
/**
 * `plugin.ui.image`. The wire form (`{ $vestraImage: path, ref }`) is an
 * internal detail; `ref` makes every call a new value, so a file rewritten
 * under the same path is sent (and decoded) again. The player checks the
 * path on every use, so a hand-made wire value gains nothing.
 */
export declare class ImageHandle implements ImageRef {
    #private;
    readonly path: string;
    readonly __vestraImage: true;
    constructor(scope: ScopeImpl, path: string, maxSide?: number);
    /** Released with its scope: writing it is `invalid`. */
    get released(): boolean;
    toJSON(): Json;
}
/** `ImageOptions.maxSide`, checked: an integer 1–4096 or absent. */
export declare function imageSide(options: ImageOptions | undefined): number | undefined;
export declare const ui: (scope: ScopeImpl) => Ui;
