import { LOCALES } from "../i18n.js";
import { THEMES } from "./appearance.js";
const PHASES = [
    "empty",
    "loading",
    "ready",
    "ended",
    "error",
];
const CONTROLS = [
    "pause",
    "seek",
    "rate",
    "restart",
    "close",
    "snapshot",
    "next",
    "previous",
];
class ConditionImpl {
    #json;
    constructor(json) {
        this.#json = json;
        Object.freeze(this);
    }
    and(...others) {
        return new ConditionImpl({
            all: [this.#json, ...others.map(conditionJson)],
        });
    }
    or(...others) {
        return new ConditionImpl({
            any: [this.#json, ...others.map(conditionJson)],
        });
    }
    not() {
        return new ConditionImpl({ not: this.#json });
    }
    toJSON() {
        return this.#json;
    }
    static json(condition) {
        return condition.#json;
    }
}
/** Whether `value` is a condition made by `plugin.when`. */
export const isCondition = (value) => value instanceof ConditionImpl;
/** A condition's wire form; throws for anything else. */
export function conditionJson(condition) {
    if (!(condition instanceof ConditionImpl))
        throw new TypeError("a condition comes from plugin.when");
    return ConditionImpl.json(condition);
}
const make = (json) => new ConditionImpl(json);
/** The one `plugin.when` (conditions hold no plugin state). */
export const WHEN = Object.freeze({
    open: make({ not: { phase: ["empty"] } }),
    ready: make({ phase: ["ready"] }),
    playing: make({ all: [{ phase: ["ready"] }, { paused: false }] }),
    paused: make({ all: [{ phase: ["ready"] }, { paused: true }] }),
    stream: make({ kind: "stream" }),
    fullscreen: make({ fullscreen: true }),
    phase(...phases) {
        if (!phases.length || phases.some((phase) => !PHASES.includes(phase)))
            throw new TypeError(`phase is one or more of ${PHASES.join(", ")}`);
        return make({ phase: [...phases] });
    },
    can(control) {
        if (!CONTROLS.includes(control))
            throw new TypeError(`can is one of ${CONTROLS.join(", ")}`);
        return make({ can: control });
    },
    theme(theme) {
        if (!THEMES.includes(theme))
            throw new TypeError(`theme is ${THEMES.map((t) => `"${t}"`).join(" or ")}`);
        return make({ theme });
    },
    language(language) {
        if (language !== "system" &&
            !LOCALES.includes(language))
            throw new TypeError(`language is "system" or one of ${LOCALES.map((l) => `"${l}"`).join(", ")}`);
        return make({ language });
    },
});
/** Evaluate a wire condition as the player does (for the test host). */
export function evaluate(json, facts) {
    if ("phase" in json)
        return json.phase.includes(facts.phase);
    if ("paused" in json)
        return facts.paused === json.paused;
    if ("kind" in json)
        return facts.kind === json.kind;
    if ("can" in json)
        return facts.can[json.can] === true;
    if ("fullscreen" in json)
        return facts.fullscreen === json.fullscreen;
    if ("theme" in json)
        return facts.theme === json.theme;
    if ("language" in json)
        return facts.language === json.language;
    if ("all" in json)
        return json.all.every((each) => evaluate(each, facts));
    if ("any" in json)
        return json.any.some((each) => evaluate(each, facts));
    return !evaluate(json.not, facts);
}
