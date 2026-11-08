import { addCapabilityEdit, CAPABILITY_DIAGNOSTIC, CAPABILITY_UNCHECKED, checkCapabilities, isPluginSource, } from "./capability-check.js";
const FIX_NAME = "vestraAddCapability";
/** The nearest manifest.json at or above a file, with its capabilities. */
function manifestFor(ts, file) {
    let directory = file.replaceAll("\\", "/");
    for (;;) {
        const parent = directory.slice(0, directory.lastIndexOf("/"));
        if (!parent || parent === directory)
            return undefined;
        directory = parent;
        const path = `${directory}/manifest.json`;
        if (!ts.sys.fileExists(path))
            continue;
        try {
            const manifest = JSON.parse(ts.sys.readFile(path) ?? "");
            if (!Array.isArray(manifest.capabilities))
                return undefined;
            return {
                path,
                directory,
                capabilities: manifest.capabilities.filter((each) => typeof each === "string"),
            };
        }
        catch {
            return undefined;
        }
    }
}
/**
 * The manifest's text for the quick fix. The TypeScript server applies a
 * code fix only to files it knows, and editors do not hand it JSON files
 * (VS Code does not), so the manifest is made known first; its text is then
 * the server's: an open buffer when the editor shares one, else the file on
 * disk (VS Code: the saved manifest, not unsaved edits).
 */
function manifestText(ts, info, path) {
    const service = info.project?.projectService;
    const script = service && ts.server?.toNormalizedPath
        ? service.getOrCreateScriptInfoForNormalizedPath(ts.server.toNormalizedPath(path), false)
        : undefined;
    const snapshot = script?.getSnapshot();
    return snapshot
        ? snapshot.getText(0, snapshot.getLength())
        : ts.sys.readFile(path);
}
/** @internal The plugin factory tsserver calls (`ts-plugin/index.js`). */
export default function init(modules) {
    const ts = modules.typescript;
    return {
        create(info) {
            const service = info.languageService;
            const log = (message) => info.project?.projectService?.logger?.info(`[@vestra-player/sdk/ts-plugin] ${message}`);
            const none = { findings: [], unchecked: [] };
            const results = (file) => {
                const manifest = manifestFor(ts, file);
                const program = service.getProgram();
                if (!manifest || !program || !isPluginSource(manifest.directory, file))
                    return none;
                try {
                    return checkCapabilities(ts, program, {
                        files: [file],
                        capabilities: manifest.capabilities,
                    });
                }
                catch (error) {
                    log(`check failed: ${error.stack ?? String(error)}`);
                    return none;
                }
            };
            const findings = (file) => results(file).findings;
            const proxy = Object.create(null);
            for (const key of Object.keys(service)) {
                const member = service[key];
                proxy[key] =
                    typeof member === "function" ? member.bind(service) : member;
            }
            proxy.getSemanticDiagnostics = (file) => {
                const prior = service.getSemanticDiagnostics(file);
                const source = service.getProgram()?.getSourceFile(file);
                const { findings: missing, unchecked } = results(file);
                return [
                    ...prior,
                    ...missing.map((finding) => ({
                        file: source,
                        start: finding.start,
                        length: finding.length,
                        messageText: finding.message,
                        category: ts.DiagnosticCategory.Error,
                        code: CAPABILITY_DIAGNOSTIC,
                        source: "vestra",
                    })),
                    ...unchecked.map((call) => ({
                        file: source,
                        start: call.start,
                        length: call.length,
                        messageText: call.message,
                        category: ts.DiagnosticCategory.Warning,
                        code: CAPABILITY_UNCHECKED,
                        source: "vestra",
                    })),
                ];
            };
            proxy.getSupportedCodeFixes = (file) => [
                ...service.getSupportedCodeFixes(file),
                String(CAPABILITY_DIAGNOSTIC),
            ];
            proxy.getCodeFixesAtPosition = (file, start, end, errorCodes, format, preferences) => {
                const prior = service.getCodeFixesAtPosition(file, start, end, errorCodes, format, preferences);
                if (!errorCodes.includes(CAPABILITY_DIAGNOSTIC))
                    return prior;
                const manifest = manifestFor(ts, file);
                if (!manifest)
                    return prior;
                const text = manifestText(ts, info, manifest.path);
                if (text === undefined)
                    return prior;
                const wanted = new Set(findings(file)
                    .filter((finding) => finding.start <= end && finding.start + finding.length >= start)
                    .map((finding) => finding.capability));
                const fixes = [];
                for (const capability of wanted) {
                    const edit = addCapabilityEdit(ts, text, capability);
                    if (!edit)
                        continue;
                    fixes.push({
                        fixName: FIX_NAME,
                        description: `Add '${capability}' to manifest.json`,
                        changes: [
                            {
                                fileName: manifest.path,
                                textChanges: [
                                    {
                                        span: { start: edit.start, length: edit.length },
                                        newText: edit.newText,
                                    },
                                ],
                            },
                        ],
                    });
                }
                return [...prior, ...fixes];
            };
            log("enabled");
            return proxy;
        },
    };
}
