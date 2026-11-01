// The agent's tools: pi's read/write/edit/ls/find on our WorkFolder, a
// pure-JS grep (pi's grep always starts ripgrep; the worker may not spawn)
// delete (so a debug file does not have to stay in the project) and the SDK
// documentation (docs, docs_search). read, ls, find and grep also reach the
// read-only reference folders (the SDK package: its .d.ts declarations and
// Slint library) by a short name, sdk/… (also their absolute path); write,
// edit and delete only the work folder.
import path from "node:path";
import {
  Type,
  createEditToolDefinition,
  createFindToolDefinition,
  createLsToolDefinition,
  createReadToolDefinition,
  createWriteToolDefinition,
  type ToolDefinition,
} from "../vendor/pi/pi.mjs";
import { DOCS_TOOL_NAMES, docsTools } from "./docs-tools.ts";
import { MAX_FILE, TooLargeError, WorkFolder, megabytes } from "./files.ts";

/** Most files one grep walks. */
const GREP_FILES = 5000;

export const TOOL_NAMES = ["read", "write", "edit", "ls", "find", "grep", "delete", ...DOCS_TOOL_NAMES];

/**
 * The folder a path belongs to: a reference folder it lies in, else the work
 * folder (which refuses it if outside). A reference folder also goes by its
 * folder name relative to the work folder (`sdk/dist/plugin/types.d.ts`, pi drops a leading
 * `@`): models copy a 100-character absolute path wrong (field test
 * 2026-10-24: 23 of 28 failed calls). Project folders are ids with a dot, so
 * the names never meet a project.
 */
export class Folders {
  readonly work: WorkFolder;
  readonly docs: WorkFolder[];
  constructor(work: WorkFolder, docs: WorkFolder[]) {
    this.work = work;
    this.docs = docs;
  }
  /** The work folder and the read-only reference folders (StartMessage.docsRoots). */
  static async open(workDir: string, docsRoots: string[]): Promise<Folders> {
    return new Folders(await WorkFolder.open(workDir), await Promise.all(docsRoots.map((root) => WorkFolder.open(root))));
  }
  /** The real path of `file`: sdk/… → the reference folder; anything else as given. */
  real(file: string): string {
    const absolute = path.resolve(this.work.root, file.replace(/^@/, ""));
    for (const doc of this.docs) {
      const alias = path.join(this.work.root, path.basename(doc.root));
      if (absolute === alias || absolute.startsWith(alias + path.sep)) return path.join(doc.root, path.relative(alias, absolute));
    }
    return file;
  }
  of(file: string): WorkFolder {
    const absolute = path.resolve(this.work.root, this.real(file));
    return this.docs.find((d) => absolute === d.root || absolute.startsWith(d.root + path.sep)) ?? this.work;
  }
  /** How the agent sees a file: relative to the work folder, or sdk/… for a reference file. */
  shown(absolute: string): string {
    const doc = this.docs.find((d) => absolute === d.root || absolute.startsWith(d.root + path.sep));
    if (!doc) return this.work.relative(absolute);
    return [path.basename(doc.root), ...path.relative(doc.root, absolute).split(path.sep)].filter(Boolean).join("/");
  }
  /** The work folder for a write; a reference path is refused. */
  writable(file: string): string {
    if (this.of(file) !== this.work) throw new Error(`The reference is read-only: ${file}`);
    return file;
  }
}

/** Glob as fd matches it: a pattern without "/" matches the file name. */
function matches(file: string, pattern: string): boolean {
  return pattern.includes("/") ? path.matchesGlob(file, pattern) : path.matchesGlob(path.posix.basename(file), pattern) || path.matchesGlob(file, pattern);
}

const grepSchema = Type.Object({
  pattern: Type.String({ description: "Regular expression (or literal text with literal: true)" }),
  path: Type.Optional(Type.String({ description: "File or folder to search, relative to the work folder (default: all), or in the reference, e.g. sdk/dist/plugin/types.d.ts" })),
  glob: Type.Optional(Type.String({ description: "Only files matching this glob, e.g. *.ts" })),
  ignoreCase: Type.Optional(Type.Boolean()),
  literal: Type.Optional(Type.Boolean()),
  limit: Type.Optional(Type.Number({ description: "Most matches to return (default 100)" })),
});

function grepTool(folders: Folders): ToolDefinition {
  return {
    name: "grep",
    label: "grep",
    description: "Search file contents in the work folder or the read-only reference (sdk/…). Returns file:line: text for each matching line.",
    promptSnippet: "Search file contents for patterns",
    parameters: grepSchema,
    async execute(_id, params: { pattern: string; path?: string; glob?: string; ignoreCase?: boolean; literal?: boolean; limit?: number }, signal) {
      const source = params.literal ? params.pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") : params.pattern;
      const regex = new RegExp(source, params.ignoreCase ? "i" : "");
      const limit = Math.max(1, Math.min(params.limit ?? 100, 1000));
      const folder = folders.of(params.path ?? ".");
      const start = await folder.resolve(folders.real(params.path ?? "."));
      const single = !(await folder.isDirectory(start));
      const files = single ? [path.basename(start)] : await folder.walk(start, GREP_FILES);
      const base = single ? path.dirname(start) : start;
      const lines: string[] = [];
      // Files it could not search: said in the result, so that "No matches"
      // never stands for "not searched" (a 1 MB cap once hid the whole
      // reference this way).
      const skipped: string[] = [];
      for (const file of files) {
        if (signal?.aborted) throw new Error("Operation aborted");
        if (params.glob && !matches(file, params.glob)) continue;
        const shown = folders.shown(path.join(base, file));
        let buffer: Buffer;
        try {
          buffer = await folder.readFile(path.join(base, file));
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code === "ENOENT") continue; // vanished
          skipped.push(error instanceof TooLargeError ? `skipped ${shown}: larger than ${megabytes(MAX_FILE)}` : `skipped ${shown}: ${(error as Error).message}`);
          continue;
        }
        if (buffer.includes(0)) {
          // Binary: named only when it was the file asked for (images in a folder are expected).
          if (single) skipped.push(`skipped ${shown}: binary file`);
          continue;
        }
        for (const [index, line] of buffer.toString("utf8").split("\n").entries()) {
          if (!regex.test(line)) continue;
          lines.push(`${shown}:${index + 1}: ${line.length > 300 ? `${line.slice(0, 300)}…` : line}`);
          if (lines.length >= limit) break;
        }
        if (lines.length >= limit) break;
      }
      const notes = [
        ...(lines.length >= limit ? [`(stopped at ${limit} matches)`] : []),
        ...(!single && files.length >= GREP_FILES ? [`(searched only the first ${GREP_FILES} files; give a narrower path)`] : []),
        ...skipped.slice(0, 20),
        ...(skipped.length > 20 ? [`(${skipped.length - 20} more files skipped)`] : []),
      ];
      const text = [lines.length ? lines.join("\n") : skipped.length ? "No matches found in the files searched" : "No matches found", ...notes].join("\n");
      return { content: [{ type: "text", text }], details: undefined };
    },
  } as ToolDefinition;
}

const deleteSchema = Type.Object({
  path: Type.String({ description: "File (or empty folder) to delete, relative to the work folder, e.g. octocat.demo/tests/debug.test.ts" }),
});

function deleteTool(folders: Folders): ToolDefinition {
  const folder = folders.work;
  return {
    name: "delete",
    label: "delete",
    description: "Delete a file, or an empty folder, in the work folder (not in the read-only reference folders). Use it for debug or scratch files and files the plugin no longer needs.",
    promptSnippet: "Delete a file in the work folder",
    parameters: deleteSchema,
    async execute(_id, params: { path: string }) {
      const removed = await folder.remove(folders.writable(params.path));
      return { content: [{ type: "text", text: `Deleted ${removed}` }], details: undefined };
    },
  } as ToolDefinition;
}

/**
 * The agent's tools: the file tools, grep, delete, the SDK documentation
 * (docs, docs_search; docs-tools.ts) and `extra`, the tools the plugin
 * process answers (the plugin tools).
 */
export function createTools(folders: Folders, extra: ToolDefinition[] = []): ToolDefinition[] {
  const folder = folders.work;
  const root = folder.root;
  const of = (file: string) => folders.of(file);
  const real = (file: string) => folders.real(file);
  const own = (file: string) => folders.writable(file);
  // access checks (path, plain file, size) without reading the file; pi's
  // read then reads it once and returns the requested lines (≤ 2000 lines / 50 KB).
  const readAny = { readFile: (file: string) => of(file).readFile(real(file)), access: async (file: string) => void (await of(file).readable(real(file))) };
  const read = { readFile: (file: string) => folder.readFile(own(file)), access: async (file: string) => void (await folder.readable(own(file))) };
  return [
    createReadToolDefinition(root, { autoResizeImages: false, operations: { ...readAny, detectImageMimeType: async () => null } }),
    createWriteToolDefinition(root, {
      operations: { writeFile: (file, content) => folder.writeFile(own(file), content), mkdir: (dir) => folder.mkdir(own(dir)) },
    }),
    createEditToolDefinition(root, { operations: { ...read, writeFile: (file, content) => folder.writeFile(own(file), content) } }),
    createLsToolDefinition(root, {
      operations: {
        exists: (file) => of(file).exists(real(file)),
        stat: async (file) => {
          const directory = await of(file).isDirectory(real(file));
          return { isDirectory: () => directory };
        },
        readdir: (dir) => of(dir).readdir(real(dir)),
      },
    }),
    createFindToolDefinition(root, {
      operations: {
        exists: (file) => of(file).exists(real(file)),
        glob: async (pattern, cwd, { ignore, limit }) =>
          (await of(cwd).walk(real(cwd), 20000)).filter((file) => matches(file, pattern) && !ignore.some((skip) => path.matchesGlob(file, skip))).slice(0, limit),
      },
    }),
    grepTool(folders),
    deleteTool(folders),
    ...docsTools(),
    ...extra,
  ] as ToolDefinition[];
}
