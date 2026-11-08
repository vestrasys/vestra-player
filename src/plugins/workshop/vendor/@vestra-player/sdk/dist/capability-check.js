import { ARGUMENT_APIS } from "./api-requests.js";
import { argumentRequests, memberNeeds, requestsNeed, RULES, } from "./capability-rules.js";
/** @internal The module plugins import the API types from. */
export const SDK_MODULE = "@vestra-player/sdk";
/** @internal The diagnostic code the editor shows (and its quick fix keys on). */
export const CAPABILITY_DIAGNOSTIC = 95_301;
/**
 * @internal The warning code the editor shows for a call whose capability
 * the checker cannot read (`plugin.wire.request` with a computed method).
 */
export const CAPABILITY_UNCHECKED = 95_302;
const memberCache = new WeakMap();
const argumentCache = new WeakMap();
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
/** The SDK's declarations of the members named (`Interface.member`). */
function resolveMembers(ts, program, apis) {
    const members = new Map();
    const checker = program.getTypeChecker();
    const module = sdkModule(ts, program, checker);
    if (!module)
        return members;
    const exports = new Map();
    for (const symbol of checker.getExportsOfModule(module))
        exports.set(symbol.name, symbol.flags & ts.SymbolFlags.Alias
            ? checker.getAliasedSymbol(symbol)
            : symbol);
    for (const api of apis) {
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
    return members;
}
/** The SDK's declarations of the members in the table. */
function sdkMembers(ts, program, rules) {
    const cached = memberCache.get(program);
    if (cached && rules === RULES)
        return cached;
    const members = resolveMembers(ts, program, Object.keys(rules.apis));
    if (rules === RULES)
        memberCache.set(program, members);
    return members;
}
/** The SDK's declarations of the members of `ARGUMENT_APIS`. */
function argumentMembers(ts, program) {
    let members = argumentCache.get(program);
    if (!members) {
        members = resolveMembers(ts, program, Object.keys(ARGUMENT_APIS));
        argumentCache.set(program, members);
    }
    return members;
}
/**
 * The strings an expression's type fixes: a string literal, a constant of
 * one, or a union of them (the caller takes one only); `undefined` when
 * the type allows any other.
 */
function literalStrings(ts, checker, node) {
    if (!node)
        return undefined;
    if (ts.isStringLiteralLike(node))
        return [node.text];
    const type = checker.getTypeAtLocation(node);
    const parts = type.isUnion() ? type.types : [type];
    const values = [];
    for (const part of parts) {
        if (!part.isStringLiteral())
            return undefined;
        values.push(part.value);
    }
    return values.length ? values : undefined;
}
/**
 * The string fields of a request's parameters that the player's rules may
 * read (`event`, `stage`): those of an object literal whose value is one
 * string the types fix.
 */
function stringFields(ts, checker, node) {
    if (!node || !ts.isObjectLiteralExpression(node))
        return undefined;
    const fields = {};
    for (const property of node.properties) {
        if (!ts.isPropertyAssignment(property))
            continue;
        const name = property.name;
        const key = ts.isIdentifier(name) || ts.isStringLiteralLike(name)
            ? name.text
            : undefined;
        const values = literalStrings(ts, checker, property.initializer);
        if (key !== undefined && values?.length === 1)
            fields[key] = values[0];
    }
    return Object.keys(fields).length ? fields : undefined;
}
/**
 * Every use, in `options.files`, of a member whose capability the manifest
 * does not declare, in file and position order.
 *
 * @internal
 */
export function findUndeclaredCapabilities(ts, program, options) {
    return checkCapabilities(ts, program, options).findings;
}
/**
 * The capability check: the uses of members whose capability the manifest does
 * not declare (`findings`, as {@link findUndeclaredCapabilities}), and the
 * calls whose capability depends on a name the checker cannot read
 * (`unchecked`).
 *
 * A call of `plugin.player.on` or `plugin.wire.on` is checked by its event
 * name, one of `plugin.wire.request` by its method name and the string
 * fields of its parameters (`{ event: "…" }`), when the types fix them.
 *
 * @internal
 */
export function checkCapabilities(ts, program, options) {
    const rules = options.rules ?? RULES;
    const findings = [];
    const unchecked = [];
    const files = options.files
        .map((file) => program.getSourceFile(file))
        .filter((file) => !!file);
    if (!files.length)
        return { findings, unchecked };
    const members = sdkMembers(ts, program, rules);
    const calls = argumentMembers(ts, program);
    if (!members.size && !calls.size)
        return { findings, unchecked };
    const checker = program.getTypeChecker();
    const apiOf = (table, symbol) => {
        for (const declaration of symbol?.declarations ?? []) {
            const api = table.get(declaration);
            if (api)
                return api;
        }
        return undefined;
    };
    for (const file of files) {
        const position = (name) => {
            const start = name.getStart(file);
            const { line, character } = file.getLineAndCharacterOfPosition(start);
            return {
                file: file.fileName,
                line: line + 1,
                column: character + 1,
                start,
                length: name.getEnd() - start,
            };
        };
        const missing = (name, api, needs, shown = api) => {
            for (const need of needs) {
                if (options.capabilities.includes(need.capability))
                    continue;
                findings.push({
                    ...position(name),
                    api,
                    capability: need.capability,
                    method: need.method,
                    ...(need.requiredBy ? { requiredBy: need.requiredBy } : {}),
                    message: need.requiredBy
                        ? `${shown} needs the capability "${need.capability}" (required by "${need.requiredBy}"), which manifest.json does not declare`
                        : `${shown} needs the capability "${need.capability}", which manifest.json does not declare`,
                });
            }
        };
        const report = (name, symbol) => {
            const api = apiOf(members, symbol);
            if (!api)
                return;
            missing(name, api, memberNeeds(api, options.capabilities, rules) ?? []);
        };
        /** A call of a member of `ARGUMENT_APIS`, by its name argument. */
        const call = (node) => {
            const callee = node.expression;
            const name = ts.isPropertyAccessExpression(callee)
                ? callee.name
                : ts.isElementAccessExpression(callee) &&
                    ts.isStringLiteralLike(callee.argumentExpression)
                    ? callee.argumentExpression
                    : undefined;
            if (!name)
                return;
            const api = apiOf(calls, checker.getSymbolAtLocation(name));
            if (!api)
                return;
            const kind = ARGUMENT_APIS[api];
            const values = literalStrings(ts, checker, node.arguments[0]);
            // One name, or not checked: a union could be any of its names at run
            // time, and none of them is known to be used.
            if (values?.length !== 1) {
                unchecked.push({
                    ...position(name),
                    api,
                    message: `${api} with ${kind === "event" ? "an event" : "a method"} name that is not one literal: its capability is not checked here (the player checks it)`,
                });
                return;
            }
            const params = kind === "method"
                ? stringFields(ts, checker, node.arguments[1])
                : undefined;
            for (const value of values) {
                const requests = argumentRequests(kind, value, params, rules);
                const shown = `${api}(${JSON.stringify(value)})`;
                if (!requests) {
                    unchecked.push({
                        ...position(name),
                        api,
                        message: `${shown}: the player's capability table has no such method; not checked`,
                    });
                    continue;
                }
                missing(name, api, requestsNeed(requests, options.capabilities, rules), shown);
            }
        };
        /**
         * A member of `ARGUMENT_APIS` taken other than by a direct call (`const
         * { request } = plugin.wire`, `const on = plugin.player.on`): its calls
         * cannot be followed, so it is unchecked where it is taken.
         */
        const taken = (name, symbol) => {
            const api = apiOf(calls, symbol);
            if (!api)
                return;
            const access = name.parent;
            const parent = access?.parent;
            if (access &&
                (ts.isPropertyAccessExpression(access) ||
                    ts.isElementAccessExpression(access)) &&
                parent &&
                ts.isCallExpression(parent) &&
                parent.expression === access)
                return; // a direct call: `call` checks it
            unchecked.push({
                ...position(name),
                api,
                message: `${api} taken other than by a direct call: the calls of it are not checked here (the player checks them)`,
            });
        };
        const visit = (node) => {
            if (ts.isCallExpression(node))
                call(node);
            if (ts.isPropertyAccessExpression(node)) {
                const symbol = checker.getSymbolAtLocation(node.name);
                report(node.name, symbol);
                taken(node.name, symbol);
            }
            else if (ts.isElementAccessExpression(node) &&
                ts.isStringLiteralLike(node.argumentExpression)) {
                const symbol = checker.getSymbolAtLocation(node.argumentExpression);
                report(node.argumentExpression, symbol);
                taken(node.argumentExpression, symbol);
            }
            else if (ts.isBindingElement(node) &&
                ts.isObjectBindingPattern(node.parent)) {
                const name = node.propertyName ?? node.name;
                const text = ts.isIdentifier(name) || ts.isStringLiteralLike(name)
                    ? name.text
                    : undefined;
                if (text !== undefined) {
                    const symbol = checker
                        .getTypeAtLocation(node.parent)
                        .getProperty(text);
                    report(name, symbol);
                    taken(name, symbol);
                }
            }
            ts.forEachChild(node, visit);
        };
        visit(file);
    }
    return { findings, unchecked };
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
