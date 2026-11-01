// The SDK documentation for the agent: docs(path) and docs_search(query),
// backed by @vestra-player/sdk/docs (SDK 1.5.50+), a tree of short pages read
// by path like `go doc`. Every answer is at most 8 KB (DOCS_PAGE_BYTES); a
// longer one ends with the number of its next page, which `page` asks for.
//
// The vendored copy is imported by its path (vendor/ is not a node_modules
// folder, and the worker must use the SDK the plugin ships, not whatever the
// player's or a checkout's node_modules hold). docs.js reads
// dist/docs-tree.json.gz next to itself, inside the plugin folder the worker
// may read (--allow-fs-read).
import { Type, type ToolDefinition } from "../vendor/pi/pi.mjs";
import { docs, searchDocs } from "../vendor/@vestra-player/sdk/dist/docs.js";

export const DOCS_TOOL_NAMES = ["docs", "docs_search"];

const page = Type.Optional(Type.Number({ description: "Page of a long answer (1-based; an answer that continues ends with the next page's number)" }));

const docsSchema = Type.Object({
  path: Type.Optional(
    Type.String({
      description:
        'Path in the documentation tree, default "/". Examples: "/", "/Guides/How-to", "/API/Commands/Commands/add", "/API/Media/Snapshot/export". Case, spaces and punctuation do not matter; "." works like "/"; a name alone finds its declaration ("TestHost", "Snapshot.export").',
    }),
  ),
  page,
});

const searchSchema = Type.Object({
  query: Type.String({ description: 'Words to look for, e.g. "add a menu item", "snapshot export", "onError"' }),
  page,
});

/** A whole page number, or undefined for the first. */
const pageOf = (value: unknown) => (typeof value === "number" && Number.isFinite(value) && value >= 1 ? Math.floor(value) : undefined);

const answer = (text: string) => ({ content: [{ type: "text" as const, text }], details: undefined });

export function docsTools(): ToolDefinition[] {
  return [
    {
      name: "docs",
      label: "docs",
      description:
        "Read the Vestra plugin SDK documentation by path. Start at \"/\" (Overview, Guides, API); every answer lists the paths one level down with a one-line summary, so drill down from there. An API member path gives its signature, parameters, return value, errors, capability, player version and examples. At most 8 KB per answer; use page for the rest.",
      promptSnippet: "Read the SDK documentation by path",
      parameters: docsSchema,
      async execute(_id: string, params: { path?: string; page?: number }) {
        return answer(docs(params.path || "/", { page: pageOf(params.page) }));
      },
    },
    {
      name: "docs_search",
      label: "docs_search",
      description:
        "Search the Vestra plugin SDK documentation by words: the matching paths, best first, each with a one-line summary; then read one with docs. Every word must occur; names rank first.",
      promptSnippet: "Search the SDK documentation",
      parameters: searchSchema,
      async execute(_id: string, params: { query: string; page?: number }) {
        return answer(searchDocs(String(params.query ?? ""), { page: pageOf(params.page) }));
      },
    },
  ] as unknown as ToolDefinition[];
}
