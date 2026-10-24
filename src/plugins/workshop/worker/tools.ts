// The agent's tools: pi's read/write/edit/ls/find on our WorkFolder, and a
// pure-JS grep (pi's grep always starts ripgrep; the worker may not spawn).
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

export const TOOL_NAMES = ["read", "write", "edit", "ls", "find", "grep"];

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

function grepTool(folder: WorkFolder): ToolDefinition {
  return {
    name: "grep",
    label: "grep",
    description: "Search file contents in the work folder. Returns file:line: text for each matching line.",
    promptSnippet: "Search file contents for patterns",
    parameters: grepSchema,
    async execute(_id, params: { pattern: string; path?: string; glob?: string; ignoreCase?: boolean; literal?: boolean; limit?: number }, signal) {
      const source = params.literal ? params.pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") : params.pattern;
      const regex = new RegExp(source, params.ignoreCase ? "i" : "");
      const limit = Math.max(1, Math.min(params.limit ?? 100, 1000));
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
        const shown = folder.relative(path.join(base, file));
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

export function createTools(folder: WorkFolder): ToolDefinition[] {
  const root = folder.root;
  const read = { readFile: (file: string) => folder.readFile(file), access: async (file: string) => void (await folder.readFile(file)) };
  return [
    createReadToolDefinition(root, { autoResizeImages: false, operations: { ...read, detectImageMimeType: async () => null } }),
    createWriteToolDefinition(root, {
      operations: { writeFile: (file, content) => folder.writeFile(file, content), mkdir: (dir) => folder.mkdir(dir) },
    }),
    createEditToolDefinition(root, { operations: { ...read, writeFile: (file, content) => folder.writeFile(file, content) } }),
    createLsToolDefinition(root, {
      operations: {
        exists: (file) => folder.exists(file),
        stat: async (file) => {
          const directory = await folder.isDirectory(file);
          return { isDirectory: () => directory };
        },
        readdir: (dir) => folder.readdir(dir),
      },
    }),
    createFindToolDefinition(root, {
      operations: {
        exists: (file) => folder.exists(file),
        glob: async (pattern, cwd, { ignore, limit }) =>
          (await folder.walk(cwd, 20000)).filter((file) => matches(file, pattern) && !ignore.some((skip) => path.matchesGlob(file, skip))).slice(0, limit),
      },
    }),
    grepTool(folder),
  ] as ToolDefinition[];
}
