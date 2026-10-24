// The agent's tools: pi's read/write/edit/ls/find on our WorkFolder, a
// pure-JS grep (pi's grep always starts ripgrep; the worker may not spawn)
// and delete (so a debug file does not have to stay in the project).
// read, ls, find and grep also reach the read-only reference folders (the
// SDK package, knowledge/); write and edit only the work folder.
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
import { MAX_FILE, type WorkFolder } from "./files.ts";

export const TOOL_NAMES = ["read", "write", "edit", "ls", "find", "grep", "delete"];

/** The folder a path belongs to: a reference folder it lies in, else the work folder (which refuses it if outside). */
export class Folders {
  readonly work: WorkFolder;
  readonly docs: WorkFolder[];
  constructor(work: WorkFolder, docs: WorkFolder[]) {
    this.work = work;
    this.docs = docs;
  }
  of(file: string): WorkFolder {
    const absolute = path.resolve(this.work.root, file);
    return this.docs.find((d) => absolute === d.root || absolute.startsWith(d.root + path.sep)) ?? this.work;
  }
}

/** Glob as fd matches it: a pattern without "/" matches the file name. */
function matches(file: string, pattern: string): boolean {
  return pattern.includes("/") ? path.matchesGlob(file, pattern) : path.matchesGlob(path.posix.basename(file), pattern) || path.matchesGlob(file, pattern);
}

const grepSchema = Type.Object({
  pattern: Type.String({ description: "Regular expression (or literal text with literal: true)" }),
  path: Type.Optional(Type.String({ description: "File or folder to search, relative to the work folder (default: all)" })),
  glob: Type.Optional(Type.String({ description: "Only files matching this glob, e.g. *.ts" })),
  ignoreCase: Type.Optional(Type.Boolean()),
  literal: Type.Optional(Type.Boolean()),
  limit: Type.Optional(Type.Number({ description: "Most matches to return (default 100)" })),
});

function grepTool(folders: Folders): ToolDefinition {
  return {
    name: "grep",
    label: "grep",
    description: "Search file contents in the work folder or a read-only reference folder (absolute path). Returns file:line: text for each matching line.",
    promptSnippet: "Search file contents for patterns",
    parameters: grepSchema,
    async execute(_id, params: { pattern: string; path?: string; glob?: string; ignoreCase?: boolean; literal?: boolean; limit?: number }, signal) {
      const source = params.literal ? params.pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") : params.pattern;
      const regex = new RegExp(source, params.ignoreCase ? "i" : "");
      const limit = Math.max(1, Math.min(params.limit ?? 100, 1000));
      const folder = folders.of(params.path ?? ".");
      const start = await folder.resolve(params.path ?? ".");
      const single = !(await folder.isDirectory(start));
      const files = single ? [path.basename(start)] : await folder.walk(start, 5000);
      const base = single ? path.dirname(start) : start;
      const lines: string[] = [];
      for (const file of files) {
        if (signal?.aborted) throw new Error("Operation aborted");
        if (params.glob && !matches(file, params.glob)) continue;
        let buffer: Buffer;
        try {
          buffer = await folder.readFile(path.join(base, file));
        } catch {
          continue; // too large or vanished
        }
        if (buffer.length > MAX_FILE || buffer.includes(0)) continue; // binary
        // Reference files by their absolute path (the model reads them by it).
        const shown = folder === folders.work ? folder.relative(path.join(base, file)) : path.join(base, file);
        for (const [index, line] of buffer.toString("utf8").split("\n").entries()) {
          if (!regex.test(line)) continue;
          lines.push(`${shown}:${index + 1}: ${line.length > 300 ? `${line.slice(0, 300)}…` : line}`);
          if (lines.length >= limit) break;
        }
        if (lines.length >= limit) break;
      }
      const text = lines.length ? lines.join("\n") + (lines.length >= limit ? `\n(stopped at ${limit} matches)` : "") : "No matches found";
      return { content: [{ type: "text", text }], details: undefined };
    },
  } as ToolDefinition;
}

const deleteSchema = Type.Object({
  path: Type.String({ description: "File (or empty folder) to delete, relative to the work folder, e.g. octocat.demo/tests/debug.test.ts" }),
});

function deleteTool(folder: WorkFolder): ToolDefinition {
  return {
    name: "delete",
    label: "delete",
    description: "Delete a file, or an empty folder, in the work folder (not in the read-only reference folders). Use it for debug or scratch files and files the plugin no longer needs.",
    promptSnippet: "Delete a file in the work folder",
    parameters: deleteSchema,
    async execute(_id, params: { path: string }) {
      const removed = await folder.remove(params.path);
      return { content: [{ type: "text", text: `Deleted ${removed}` }], details: undefined };
    },
  } as ToolDefinition;
}

export function createTools(folders: Folders, extra: ToolDefinition[] = []): ToolDefinition[] {
  const folder = folders.work;
  const root = folder.root;
  const of = (file: string) => folders.of(file);
  const readAny = { readFile: (file: string) => of(file).readFile(file), access: async (file: string) => void (await of(file).readFile(file)) };
  const read = { readFile: (file: string) => folder.readFile(file), access: async (file: string) => void (await folder.readFile(file)) };
  return [
    createReadToolDefinition(root, { autoResizeImages: false, operations: { ...readAny, detectImageMimeType: async () => null } }),
    createWriteToolDefinition(root, {
      operations: { writeFile: (file, content) => folder.writeFile(file, content), mkdir: (dir) => folder.mkdir(dir) },
    }),
    createEditToolDefinition(root, { operations: { ...read, writeFile: (file, content) => folder.writeFile(file, content) } }),
    createLsToolDefinition(root, {
      operations: {
        exists: (file) => of(file).exists(file),
        stat: async (file) => {
          const directory = await of(file).isDirectory(file);
          return { isDirectory: () => directory };
        },
        readdir: (dir) => of(dir).readdir(dir),
      },
    }),
    createFindToolDefinition(root, {
      operations: {
        exists: (file) => of(file).exists(file),
        glob: async (pattern, cwd, { ignore, limit }) =>
          (await of(cwd).walk(cwd, 20000)).filter((file) => matches(file, pattern) && !ignore.some((skip) => path.matchesGlob(file, skip))).slice(0, limit),
      },
    }),
    grepTool(folders),
    deleteTool(folder),
    ...extra,
  ] as ToolDefinition[];
}
