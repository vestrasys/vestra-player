/**
 * System dialogs (`plugin.dialogs`): the player shows them; this side
 * checks the texts and fills their templates (L1, a service;
 * doc/specs/sdk-refactor.md §2.2 in the player repository).
 */
import { isTextValue } from "../../i18n.js";
import { failure } from "../errors.js";
import { shownText, templateArgs } from "../osd.js";
export const dialogs = (scope) => {
    const open = async (kind, options = {}) => {
        const result = await scope.runtime.host.dialogs.open({
            kind,
            ...(options.title === undefined
                ? {}
                : { title: shownText(options.title, undefined, "title") }),
            ...(options.fileName === undefined
                ? {}
                : { fileName: shownText(options.fileName, undefined, "fileName") }),
        });
        return result.cancelled ? [] : result.paths;
    };
    return {
        async confirm(options) {
            const { message, detail, confirm, cancel, destructive, checkbox, args } = options ?? {};
            const values = templateArgs(args);
            // A string's length is checked here; a text value's (shown) length by
            // the player.
            const length = (text, min, max) => isTextValue(text) ||
                (typeof text === "string" &&
                    [...text].length >= min &&
                    [...text].length <= max);
            if (!length(message, 1, 200) ||
                !length(confirm, 1, 40) ||
                (detail !== undefined && !length(detail, 0, 2000)) ||
                (cancel !== undefined && !length(cancel, 1, 40)) ||
                (checkbox !== undefined && !length(checkbox, 1, 40)) ||
                (destructive !== undefined && typeof destructive !== "boolean"))
                throw failure("invalid", "confirm needs message 1–200, confirm/cancel/checkbox 1–40, detail ≤ 2000 characters");
            // `args` fills the string templates; a text value carries its own.
            const shown = (text, what) => shownText(text, undefined, what);
            const answer = await scope.runtime.host.dialogs.confirm({
                message: shown(message, "message"),
                confirm: shown(confirm, "confirm"),
                ...(detail === undefined ? {} : { detail: shown(detail, "detail") }),
                ...(cancel === undefined ? {} : { cancel: shown(cancel, "cancel") }),
                ...(destructive === undefined ? {} : { destructive }),
                ...(checkbox === undefined
                    ? {}
                    : { checkbox: shown(checkbox, "checkbox") }),
                ...(values === undefined ? {} : { args: values }),
            });
            return {
                confirmed: answer.confirmed === true,
                checked: answer.checked === true,
            };
        },
        openFile: async (options) => (await open("openFile", options))[0] ?? null,
        openFiles: (options) => open("openFiles", options),
        openDirectory: async (options) => (await open("openDirectory", options))[0] ?? null,
        saveFile: async (options) => (await open("saveFile", options))[0] ?? null,
    };
};
