import { memberNeeds, RULES } from "./capability-rules.js";
/** @internal The module plugins import the API types from. */
export const SDK_MODULE = "@vestra-player/sdk";
/** @internal The diagnostic code the editor shows (and its quick fix keys on). */
export const CAPABILITY_DIAGNOSTIC = 95_301;
const memberCache = new WeakMap();
/**
 * The module `@vestra-player/sdk` as the program resolved it: from the
 * first import or export of it in the program's own (non-declaration) files.
 */
function sdkModule(ts, program, checker) {
    for (const file of program.getSourceFiles()) {
        if (file.isDeclarationFile || program.isSourceFileFromExternalLibrary(file))
            continue;
        for (const statement of file.statements) {
            const specifier = ts.isImportDeclaration(statement) || ts.isExportDeclaration(statement)
                ? statement.moduleSpecifier
                : undefined;
            if (specifier &&
                ts.isStringLiteral(specifier) &&
                specifier.text === SDK_MODULE) {
                const module = checker.getSymbolAtLocation(specifier);
                if (module)
                    return module;
            }
        }
    }
    return undefined;
}
/** The SDK's declarations of the members in the table. */
function sdkMembers(ts, program, rules) {
    const cached = memberCache.get(program);
    if (cached && rules === RULES)
        return cached;
    const members = new Map();
    const checker = program.getTypeChecker();
    const module = sdkModule(ts, program, checker);
    if (module) {
        const exports = new Map();
        for (const symbol of checker.getExportsOfModule(module))
            exports.set(symbol.name, symbol.flags & ts.SymbolFlags.Alias
                ? checker.getAliasedSymbol(symbol)
                : symbol);
        for (const api of Object.keys(rules.apis)) {
            const [head, ...path] = api.split(".");
            const root = exports.get(head);
            if (!root || !path.length)
                continue;
            let type = checker.getDeclaredTypeOfSymbol(root);
            let property;
            for (const name of path) {
                property = type?.getProperty(name);
                type = property && checker.getTypeOfSymbol(property);
            }
            for (const declaration of property?.declarations ?? [])
                if (!members.has(declaration))
                    members.set(declaration, api);
        }
    }
    if (rules === RULES)
        memberCache.set(program, members);
    return members;
}
/**
 * Every use, in `options.files`, of a member whose capability the manifest
 * does not declare, in file and position order.
 *
 * @internal
 */
export function findUndeclaredCapabilities(ts, program, options) {
    const rules = options.rules ?? RULES;
    const files = options.files
        .map((file) => program.getSourceFile(file))
        .filter((file) => !!file);
    if (!files.length)
        return [];
    const members = sdkMembers(ts, program, rules);
    if (!members.size)
        return [];
    const checker = program.getTypeChecker();
    const findings = [];
    const memberOf = (symbol) => {
        for (const declaration of symbol?.declarations ?? []) {
            const api = members.get(declaration);
            if (api)
                return api;
        }
        return undefined;
    };
    for (const file of files) {
        const report = (name, symbol) => {
            const api = memberOf(symbol);
            if (!api)
                return;
            const start = name.getStart(file);
            const { line, character } = file.getLineAndCharacterOfPosition(start);
            for (const need of memberNeeds(api, options.capabilities, rules) ?? []) {
                if (options.capabilities.includes(need.capability))
                    continue;
                findings.push({
                    file: file.fileName,
                    line: line + 1,
                    column: character + 1,
                    start,
                    length: name.getEnd() - start,
                    api,
                    capability: need.capability,
                    method: need.method,
                    ...(need.requiredBy ? { requiredBy: need.requiredBy } : {}),
                    message: need.requiredBy
                        ? `${api} needs the capability "${need.capability}" (required by "${need.requiredBy}"), which manifest.json does not declare`
                        : `${api} needs the capability "${need.capability}", which manifest.json does not declare`,
                });
            }
        };
        const visit = (node) => {
            if (ts.isPropertyAccessExpression(node))
                report(node.name, checker.getSymbolAtLocation(node.name));
            else if (ts.isElementAccessExpression(node) &&
                ts.isStringLiteralLike(node.argumentExpression))
                report(node.argumentExpression, checker.getSymbolAtLocation(node.argumentExpression));
            else if (ts.isBindingElement(node) &&
                ts.isObjectBindingPattern(node.parent)) {
                const name = node.propertyName ?? node.name;
                const text = ts.isIdentifier(name) || ts.isStringLiteralLike(name)
                    ? name.text
                    : undefined;
                if (text !== undefined)
                    report(name, checker.getTypeAtLocation(node.parent).getProperty(text));
            }
            ts.forEachChild(node, visit);
        };
        visit(file);
    }
    return findings;
}
/**
 * Whether a file belongs to the plugin's production code, given the
 * directory of its `manifest.json`: not in `tests/`, `node_modules/`,
 * `dist/`, `vendor/` or a dot directory (what `vestra-plugin build` leaves
 * out), and not a declaration file.
 *
 * @internal
 */
export function isPluginSource(directory, file) {
    const normal = (path) => path.replaceAll("\\", "/").replace(/\/$/, "");
    const base = normal(directory);
    const path = normal(file);
    const caseless = /^[a-z]:\//i.test(path);
    const same = (a, b) => caseless ? a.toLowerCase() === b.toLowerCase() : a === b;
    if (!same(path.slice(0, base.length + 1), `${base}/`))
        return false;
    if (/\.d\.[mc]?ts$/.test(path) || !/\.[mc]?tsx?$/.test(path))
        return false;
    const parts = path
        .slice(base.length + 1)
        .split("/")
        .slice(0, -1);
    return !parts.some((part) => part.startsWith(".") ||
        ["tests", "node_modules", "dist", "vendor"].includes(part));
}
/**
 * The edit that adds a capability to a manifest's `capabilities` array, in
 * the array's own layout; `undefined` when the text has no such array.
 *
 * @internal
 */
export function addCapabilityEdit(ts, text, capability) {
    const json = ts.parseJsonText("manifest.json", text);
    const root = json.statements[0]?.expression;
    if (!root || !ts.isObjectLiteralExpression(root))
        return undefined;
    const property = root.properties.find((each) => ts.isPropertyAssignment(each) &&
        ts.isStringLiteral(each.name) &&
        each.name.text === "capabilities");
    const array = property?.initializer;
    if (!array || !ts.isArrayLiteralExpression(array))
        return undefined;
    const quoted = JSON.stringify(capability);
    const last = array.elements.at(-1);
    if (!last) {
        const start = array.getStart(json);
        return {
            start,
            length: array.getEnd() - start,
            newText: `[${quoted}]`,
        };
    }
    const first = array.elements[0];
    const lineStart = text.lastIndexOf("\n", first.getStart(json)) + 1;
    const multiline = text
        .slice(array.getStart(json), first.getStart(json))
        .includes("\n");
    const indent = text.slice(lineStart, first.getStart(json));
    return {
        start: last.getEnd(),
        length: 0,
        // The manifest's own line endings (CRLF on many Windows checkouts).
        newText: multiline
            ? `,${text.includes("\r\n") ? "\r\n" : "\n"}${indent}${quoted}`
            : `, ${quoted}`,
    };
}
