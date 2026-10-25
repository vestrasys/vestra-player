# Markdown example (examples.markdown)

A demonstration of the core Markdown display component ([S4 proposal §5](../../../doc/architecture/store-api-proposal.md)).

- Menu: 示例 ▶ Markdown 示例… (Examples ▶ Markdown Example…) opens a window that shows `sample.md`.
- The window `ui/document.slint` has only one `MarkdownView`; `main.ts` builds the property value with `plugin.ui.markdown(text, { base: <package directory> })`.
- Relative images resolve against the package directory (`assets/icon.png`); `https:` images are downloaded by the SDK to `.vestra-cache/markdown/` in the data directory and then shown; unsupported formats such as SVG show their alt text; inline HTML is not shown.
- Links: `http(s)` addresses open in the browser (`system.openUrl`); `#anchor` links only show an on-screen message.
- The manifest also demonstrates the display fields (`name`, `author`, `punchline`, `description`, `tags`, `icon`, `license`) and `minPlayerVersion`.

Development: in `src/`, `npm run plugin -- check|test|build examples/markdown`; run with `cd host && cargo run --bin vestra-host -- --plugin ../examples/markdown`.
