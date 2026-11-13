/**
 * The capability scan: which boundary categories a plugin's code uses
 * (`player`, `plugins`, `clipboard`, `credentials`, `camera`, `microphone`,
 * `screen`, `notifications`), read from its source. `vestra-plugin scan`
 * prints the result; the plugin repository's index and the store show it.
 *
 * The table is `capability-scan.json` at the package's root, maintained by
 * hand: every method of the public API and every method of the player's
 * capability table is in one category's list or in `none`'s
 * (test/capability-scan.test.ts). Nothing here is read at run time: the
 * manifest's capabilities gate the API as before.
 *
 * TypeScript files are scanned by type, with the capability checker's
 * resolution (`findMemberUses`, capability-check.ts): a member counts
 * however its object was reached. JavaScript files have no types and are
 * scanned by name, best effort (`scanNamesInto`); the result says which
 * way each file was read.
 *
 * The TypeScript module is passed in, never imported (as in
 * capability-check.ts). Node only: it reads the table from disk.
 *
 * @internal
 */
import { readFileSync } from "node:fs";
import { findMemberUses } from "./capability-check.js";
/** @internal The scan's categories, in the order results list them. */
export const SCAN_CATEGORIES = [
    "player",
    "plugins",
    "clipboard",
    "credentials",
    "camera",
    "microphone",
    "screen",
    "notifications",
];
/** @internal The version of the result's format (`scan`). */
export const SCAN_FORMAT = 1;
let table;
/**
 * `capability-scan.json`, read once from the package's root (one level
 * above this module, in `src/` and in `dist/` alike).
 *
 * @internal
 */
export function scanTable() {
    table ??= JSON.parse(readFileSync(new URL("../capability-scan.json", import.meta.url), "utf8"));
    return table;
}
/** The category of a member; `undefined` for `none` and for unknown names. */
function memberCategories(scan) {
    const categories = new Map();
    for (const category of SCAN_CATEGORIES)
        for (const member of scan.categories[category].members)
            categories.set(member, category);
    return categories;
}
/**
 * The category of a request by name, as the player routes a method: the
 * exact name first, then the first `prefix.*` entry it starts with.
 * `null`: in `none`; `undefined`: the table does not know the method.
 *
 * @internal
 */
export function requestCategory(method, scan = scanTable()) {
    const lists = [
        ...SCAN_CATEGORIES.map((category) => [
            category,
            scan.categories[category].requests,
        ]),
        [null, scan.none.requests],
    ];
    for (const [category, requests] of lists)
        if (requests.includes(method))
            return category;
    for (const [category, requests] of lists)
        if (requests.some((entry) => entry.endsWith(".*") && method.startsWith(entry.slice(0, -1))))
            return category;
    return undefined;
}
/** Collects evidence and the requests the scan cannot place. */
class Findings {
    evidence = new Map();
    unchecked = [];
    #scan;
    #relative;
    constructor(scan, relative) {
        this.#scan = scan;
        this.#relative = relative;
    }
    add(category, evidence) {
        const list = this.evidence.get(category) ?? [];
        this.evidence.set(category, list);
        list.push({ ...evidence, file: this.#relative(evidence.file) });
    }
    request(method, file, line, by) {
        if (method === undefined) {
            this.unchecked.push({
                file: this.#relative(file),
                line,
                message: "a request whose method name is not one literal string: the scan cannot tell what it asks for",
            });
            return;
        }
        const category = requestCategory(method, this.#scan);
        if (category === undefined)
            this.unchecked.push({
                file: this.#relative(file),
                line,
                message: `a request of ${JSON.stringify(method)}, which the player's capability table does not know`,
            });
        else if (category !== null)
            this.add(category, { request: method, file, line, by });
    }
    result() {
        const order = (a, b) => a.file.localeCompare(b.file, "en") ||
            a.line - b.line ||
            (a.member ?? a.request ?? "").localeCompare(b.member ?? b.request ?? "", "en");
        const uses = SCAN_CATEGORIES.filter((category) => this.evidence.has(category));
        return {
            uses,
            evidence: Object.fromEntries(uses.map((category) => [
                category,
                this.evidence.get(category).toSorted(order),
            ])),
            unchecked: this.unchecked.toSorted((a, b) => a.file.localeCompare(b.file, "en") || a.line - b.line),
        };
    }
}
/** `Clipboard` → `clipboard`: the name a module has on `plugin`. */
const lowerFirst = (name) => name[0].toLowerCase() + name.slice(1);
/**
 * A file's uses, by name (best effort, no types; for JavaScript, and for
 * TypeScript that imports no SDK types): a property
 * access or a destructured property named as a category's member, on
 * something named as the member's interface is on `plugin`
 * (`plugin.clipboard.readText`, `clipboard.readText`,
 * `const { readText } = plugin.clipboard`; `Clipboard.readText` is on
 * `clipboard`); and a call of something named `request` with one string
 * literal, as a request by name. Missed: an object reached under another
 * name, a computed member name, a member whose interface is not a module
 * of `plugin` (`PackageCandidate.install`). A name that only looks like
 * one is counted.
 */
function scanNamesInto(ts, fileName, text, scan, into) {
    // The kind (JavaScript, TypeScript) is the file name's.
    const source = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true);
    // "clipboard.readText" → category and member.
    const pairs = new Map();
    for (const [member, category] of memberCategories(scan)) {
        const parts = member.split(".");
        const name = parts.at(-1);
        const owner = parts.length > 2 ? parts.at(-2) : lowerFirst(parts[0]);
        pairs.set(`${owner}.${name}`, { category, member });
    }
    const line = (node) => source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
    /** The last name of an expression: `plugin.clipboard` → `clipboard`. */
    const lastName = (node) => {
        if (!node)
            return undefined;
        if (ts.isIdentifier(node))
            return node.text;
        if (ts.isPropertyAccessExpression(node))
            return node.name.text;
        if (ts.isElementAccessExpression(node) &&
            ts.isStringLiteralLike(node.argumentExpression))
            return node.argumentExpression.text;
        if (ts.isParenthesizedExpression(node) ||
            ts.isNonNullExpression(node) ||
            ts.isAwaitExpression(node))
            return lastName(node.expression);
        return undefined;
    };
    const member = (owner, name, at) => {
        const found = owner === undefined ? undefined : pairs.get(`${owner}.${name}`);
        if (found)
            into.add(found.category, {
                member: found.member,
                file: fileName,
                line: line(at),
                by: "names",
            });
    };
    const visit = (node) => {
        if (ts.isPropertyAccessExpression(node))
            member(lastName(node.expression), node.name.text, node.name);
        else if (ts.isElementAccessExpression(node) &&
            ts.isStringLiteralLike(node.argumentExpression))
            member(lastName(node.expression), node.argumentExpression.text, node.argumentExpression);
        else if (ts.isBindingElement(node) &&
            ts.isObjectBindingPattern(node.parent)) {
            const name = node.propertyName ?? node.name;
            const declaration = node.parent.parent;
            const from = ts.isVariableDeclaration(declaration) ||
                ts.isParameter(declaration) ||
                ts.isBindingElement(declaration)
                ? ts.isBindingElement(declaration)
                    ? // `{ clipboard: { readText } }`: the outer property's name.
                        (declaration.propertyName ?? declaration.name)
                    : declaration.initializer
                : undefined;
            if (ts.isIdentifier(name) || ts.isStringLiteralLike(name))
                member(lastName(from), name.text, name);
        }
        if (ts.isCallExpression(node) && lastName(node.expression) === "request") {
            const first = node.arguments[0];
            into.request(first && ts.isStringLiteralLike(first) ? first.text : undefined, fileName, line(node.expression), "names");
        }
        ts.forEachChild(node, visit);
    };
    visit(source);
}
/**
 * The categories a plugin's files use.
 *
 * `resolved` is `false` when there are TypeScript files and the program
 * resolves no import of `@vestra-player/sdk` in them (the SDK is not
 * installed where the plugin looks, or nothing imports it): the caller
 * then scans those files by name too, or reports it.
 *
 * @param program - A program that has the `typed` files; `undefined` with
 *   none.
 *
 * @internal
 */
export function scan(ts, program, options) {
    const table = options.table ?? scanTable();
    const findings = new Findings(table, options.relative ?? ((file) => file));
    let resolved = true;
    if (program && options.typed.length) {
        const categories = memberCategories(table);
        const found = findMemberUses(ts, program, {
            files: options.typed,
            apis: categories.keys(),
        });
        resolved = found.resolved;
        for (const use of found.uses)
            findings.add(categories.get(use.api), {
                member: use.api,
                file: use.file,
                line: use.line,
                by: "types",
            });
        for (const request of found.requests)
            findings.request(request.method, request.file, request.line, "types");
    }
    for (const { file, text } of options.named)
        scanNamesInto(ts, file, text, table, findings);
    return { ...findings.result(), resolved };
}
