/**
 * `plugin.ui`: image and Markdown values for view properties (L1, a
 * primitive; doc/specs/sdk-refactor.md §2.2 in the player repository).
 * Commands take the same images as bar button icons.
 */
import { failure } from "../errors.js";
import { markdown, remoteImageFile } from "../markdown.js";
let images = 0;
/**
 * `plugin.ui.image`. The wire form (`{ $vestraImage: path, ref }`) is an
 * internal detail; `ref` makes every call a new value, so a file rewritten
 * under the same path is sent (and decoded) again. The player checks the
 * path on every use, so a hand-made wire value gains nothing.
 */
export class ImageHandle {
    path;
    __vestraImage = true;
    #scope;
    #ref = ++images;
    #maxSide;
    constructor(scope, path, maxSide) {
        this.#scope = scope;
        this.path = path;
        this.#maxSide = maxSide;
    }
    /** Released with its scope: writing it is `invalid`. */
    get released() {
        return this.#scope.disposed;
    }
    toJSON() {
        // `maxSide` only when given: players before 0.5.1 refuse the field.
        return this.#maxSide === undefined
            ? { $vestraImage: this.path, ref: this.#ref }
            : { $vestraImage: this.path, ref: this.#ref, maxSide: this.#maxSide };
    }
}
/** `ImageOptions.maxSide`, checked: an integer 1–4096 or absent. */
export function imageSide(options) {
    const side = options?.maxSide;
    if (side === undefined)
        return undefined;
    if (!Number.isInteger(side) || side < 1 || side > 4096)
        throw failure("invalid", "maxSide must be an integer 1–4096");
    return side;
}
export const ui = (scope) => ({
    image(path, options) {
        if (typeof path !== "string" || !path)
            throw failure("invalid", "ui.image needs a file path");
        return new ImageHandle(scope, path, imageSide(options));
    },
    async remoteImage(url, options) {
        if (typeof url !== "string" || !url.startsWith("https://"))
            throw failure("invalid", "ui.remoteImage needs an https: address");
        const side = imageSide(options);
        const file = await remoteImageFile(scope.runtime, url);
        return new ImageHandle(scope, file, side);
    },
    markdown: (text, options) => markdown(scope, text, options),
});
