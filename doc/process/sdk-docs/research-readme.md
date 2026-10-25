# @vestra-player/sdk README / CONTRIBUTING rewrite research

Date: 2026-10-22. Read-only research; no repository files were changed. The original READMEs were fetched to `scratchpad/readmes/`.

---

## 0. Current state

**SDK repository** (`scratchpad/sdkrepo`, 1.2.0)

- `README.md`: 73 lines, plain text. No logo, no badges, no screenshot. In order, it contains a one-sentence introduction, a link to the API docs, installation (with a paragraph explaining Node 26), a minimal plugin, CLI usage, an entry point table, a version table, the license, and finally Pages/CI notes written for maintainers.
  - Problem one: there is no hero section at the top, so readers cannot tell which product this is an SDK for. It only links to the `github.com/vestrasys` organization page; there is no player introduction and no screenshot.
  - Problem two: three kinds of readers are mixed on one page. Plugin authors need installation and quick start, advanced users need the entry points and the version matrix, maintainers need Pages and CI.
  - Problem three: the 1.1 cell of the `Versions` table is stuffed with a long list of API names, which belongs in the CHANGELOG.
  - Problem four: TypeDoc also uses the README as the docs site home page (`"readme": "README.md"` in `typedoc.json`), so any HTML must render correctly in all three places: GitHub, npm and TypeDoc.
- `CONTRIBUTING.md`: 85 lines, titled "Maintaining @vestra-player/sdk". All of it is an internal handbook for maintainers: build commands, TypeDoc rules, CI topology diagram, release process, one-time setup of npm Trusted Publishing, syncing with the player. None of what external contributors need is there: how to file an issue, PR rules, commit conventions, code of conduct, license statement.
- `docs/getting-started.md`: a complete getting started guide. **One outdated item found**: line 12 still says "Node 22.18 or newer", while the engines field in package.json and the README already say Node 26.
- There is no `examples/` directory. The CLI only has `check / test / build / pack`; there is no `init` or scaffolding.
- Workflow file names: `ci.yml` (name: CI, triggered on non-main branches and PRs), `main.yml` (name: Main, triggered on push to main and deploys Pages), `release.yml`.

**Logo**: `/Users/ziyang/Develop/vestra-player/src/assets/brand/logo.png`. A 1024×1024 RGBA PNG, 368 KB, a rounded "V" with an orange→pink→purple→blue gradient. The gradient itself stands out on both dark and light backgrounds, **so separate dark and light logos are not needed**. This assumes the background really is transparent; I only inferred that from the RGBA format and did not verify it pixel by pixel, so please check it when copying. The SDK repository currently has no logo file.

**Examples**: the player repository has 4 in `src/examples/`: `bookmark` (window, events, playback control), `core-ui` (mount, osd, theme), `markdown` (ui.window and markdown; the manifest has Chinese display fields and an icon), `panel-menu` (ui.mount panel). Each has a README, manifest, main.ts and tests/, in complete form, and can be used directly as the source for the SDK repository's `examples/`.

---

## 1. Notes per repository

| # | Repository | Source |
|---|---|---|
| 1 | vitejs/vite | <https://raw.githubusercontent.com/vitejs/vite/main/README.md>, npm version <https://raw.githubusercontent.com/vitejs/vite/main/packages/vite/README.md> |
| 2 | slint-ui/slint | <https://raw.githubusercontent.com/slint-ui/slint/master/README.md> |
| 3 | tauri-apps/tauri | <https://raw.githubusercontent.com/tauri-apps/tauri/dev/README.md> |
| 4 | astral-sh/uv | <https://raw.githubusercontent.com/astral-sh/uv/main/README.md> |
| 5 | oven-sh/bun | <https://raw.githubusercontent.com/oven-sh/bun/main/README.md> |
| 6 | denoland/deno | <https://raw.githubusercontent.com/denoland/deno/main/README.md> |
| 7 | figma/plugin-typings | <https://raw.githubusercontent.com/figma/plugin-typings/master/README.md> |
| 8 | obsidianmd/obsidian-api and obsidian-sample-plugin | <https://raw.githubusercontent.com/obsidianmd/obsidian-api/master/README.md>, <https://raw.githubusercontent.com/obsidianmd/obsidian-sample-plugin/master/README.md> |
| 9 | microsoft/vscode-extension-samples and vscode-generator-code | <https://raw.githubusercontent.com/microsoft/vscode-extension-samples/main/README.md>, <https://raw.githubusercontent.com/microsoft/vscode-generator-code/main/README.md> |
| 10 | raycast/extensions | <https://raw.githubusercontent.com/raycast/extensions/main/README.md> |
| 11 | mpv-player/mpv | <https://raw.githubusercontent.com/mpv-player/mpv/master/README.md> |
| 12 | obsproject/obs-plugintemplate | <https://raw.githubusercontent.com/obsproject/obs-plugintemplate/master/README.md> |
| Extra | tauri-apps/plugins-workspace | <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/README.md> |

**1. Vite (66 lines).** The hero section most worth copying.
- The outer element is `<p align="center">`, containing a `<picture>` with dark and light logos, 60px high.
- Below it, one row of 4 centered badges: npm version `img.shields.io/npm/v/vite`, Node version `img.shields.io/node/v/vite`, the native GitHub CI badge `actions/workflows/ci.yml/badge.svg?branch=main`, Discord.
- Then an H1 with an emoji, a one-line blockquote tagline, 6 bullet points with emoji, two paragraphs of introduction, and finally a "Read the Docs" link.
- Further down: a Packages table (each row has an npm version badge), a one-line Contribution, a one-line License, sponsors.
- **Key point: npm uses a different README** (`packages/vite/README.md`, 20 lines). It has no logo, no badges and no HTML, only the title, tagline, bullet points and docs link.
- The README has no install commands; installation and usage are all left to the docs site.

**2. Slint (about 240 lines).**
- It starts with `![](logo-light.svg#gh-light-mode-only) ![](logo-dark.svg#gh-dark-mode-only)` to switch between dark and light logos. This is the old approach, which GitHub no longer recommends, and on npm both images are shown.
- 3 badges: CI, REUSE, Discussions.
- A bold one-sentence positioning statement, then one sentence on usage.
- "Why Slint" as bullet points. Examples lays out screenshots and videos in a table (Windows/macOS/Linux screenshots, WASM demo), linking to `examples/`.
- Hello World is a code block of about 10 lines, followed by docs links, then a fairly long architecture introduction, License, FAQ, company introduction.
- All image links use reference style `[#4]: https://...`, which keeps the table source tidy.

**3. Tauri (96 lines).**
- It starts with a full splash image (`.github/splash.png`) instead of a separate logo.
- 8 badges (status, license, CI, FOSSA, Discord, website etc.), on the many side.
- Getting Started is a single line, `npm create tauri-app@latest`, with a link to the docs.
- Features are bullet points; platform support is a table. The Contributing section links to `.github/CONTRIBUTING.md`.

**4. uv (326 lines).**
- Below the title: 3 badges (PyPI version, Python version, Discord), a one-sentence tagline, then a `<picture>` with dark and light benchmark images.
- Highlights are bullet points, each linking to the docs. Installation gives several install methods, each in a small code block.
- Features gives 5 to 10 lines of example per subcommand. Then a Contributing paragraph, FAQ, License.
- On the long side, but every section is short, so it is cheap to skim.

**5. Bun (446 lines).**
- Centered logo (170px), H1 also centered. 3 badges: Discord, stars, "speed: fast".
- Below that, a centered navigation row: Documentation • Discord • Issues • Roadmap, followed by "Read the docs →".
- "What is Bun" demonstrates the core commands in short code blocks, then Install.
- After that, 200+ lines list Quick links and Guides as a table of contents. This part is a pure wall of links; not recommended to copy.

**6. Deno (116 lines).**
- Badges use reference-style links collected at the end of the file, which keeps the body clean.
- The logo uses `<img align="right" height=150>` to float to the right of the introduction paragraph, a way to not take up height on the first screen.
- The structure, in order: Installation (one command per platform), "Your first Deno program" (3 lines of code plus 1 run command), Additional resources (4 bold links), a one-line Contributing. This is **the standard example of "what it is, install, get it running, where to read more"**.

**7. Figma plugin-typings (33 lines).** The most similar to ours: a "host API type package".
- Title plus one npm badge, one sentence of introduction.
- Usage is two numbered steps (install, tsconfig configuration); About is one paragraph on version correspondence.
- Minimal, but lacks a prominent entry to the plugin docs.

**8. Obsidian.**
- The `obsidian-api` README (69 lines) is plain text: docs link, template link, issues go to the forum, then bullet points on manifest fields and App architecture; no badges and no logo.
- `obsidian-sample-plugin` is a **GitHub Template repository**: "Use this template", clone, `npm i`, `npm run dev`, plus the release and listing process.
- In other words: **the type package only covers "what it is, where to read the docs"; the getting started flow lives in the template repository**.

**9. VS Code.**
- The title of `vscode-extension-samples` is `<h1 align="center">`, followed by a paragraph of explanation and a promise that every sample has a description, a GIF or screenshot, a guide link and a list of the APIs it uses.
- The body first lists 4 hello-world variants under "Getting Started", then a large table: Sample | official guide | APIs used. A script generates the table between `<!-- SAMPLES_BEGIN -->` and `<!-- SAMPLES_END -->`.
- Each sample is a separate top-level directory, all named `*-sample`.
- `vscode-generator-code` (yo code) has 4 badges: npm version, monthly downloads, CI, License. Usage is written as "install, `yo code`, screenshot", followed by a full text block of CLI options.

**10. Raycast extensions (31 lines).**
- Centered logo (128px) and centered H1, two large `style=for-the-badge` badges (X, Slack), one sentence of introduction, one large header image.
- Getting Started only links to developers.raycast.com. CONTRIBUTING is only 10 lines, linking to "Create your first extension" and "Contribute to an extension" on the docs site.

**11. mpv (222 lines).** Represents the "old-school" style.
- A 128px logo at the top, below it a table of contents and a list of external links (Wiki, User Scripts, FAQ, Manual).
- The body is Overview, system requirements, compiling, release cycle, bug reports, Contributing, License.
- No badges; aimed at users and packagers. Of little reference value for an SDK; only the restrained "logo + external link list" approach is worth borrowing.

**12. OBS plugin template (59 lines).**
- A template repository with no logo and no badges; a table lists the build environments, and Quick Start links to the wiki.
- The rest covers CI and signing, from a maintainer's point of view. Worth borrowing: "the README only points the way; details go in the wiki or docs".

**Extra: tauri plugins-workspace.** Lists all plugins in one "plugin name | one-sentence description" table, each row linking to a subdirectory. This is a good shape for an examples/ index.

### CONTRIBUTING sources

| Repository | Source | Structure and key points |
|---|---|---|
| Vite | <https://raw.githubusercontent.com/vitejs/vite/main/CONTRIBUTING.md> | 382 lines. In order: Repo Setup, Documentation, dependency principles ("think before adding a dependency"), "stop adding new options", Debugging, Running Tests, **AI Policy**, **Pull Request Guidelines** (new features need an issue first and must come with tests; bug fixes put `fix #123` in the title; pure style refactors are not accepted; auto-formatting on commit; PR titles follow the commit convention), Maintenance (issue triage, PR review), Release. |
| Tauri | <https://raw.githubusercontent.com/tauri-apps/tauri/dev/.github/CONTRIBUTING.md> | 120 lines. Opens by citing the Code of Conduct, with a small table of contents. The body is Issue Reporting (reproduction steps required; "need repro" closes after 5 days without reply), PR Guidelines (commits must be signed; squash merge; new features need an issue first), Development Guide, AI Tool Policy, Financial Contribution. |
| Slint | <https://raw.githubusercontent.com/slint-ui/slint/master/CONTRIBUTING.md> | 63 lines, **the size best suited for us to follow**. In order: welcome and Discussions link, good first issue label, AGENTS.md note; Pull Requests (small and focused; discuss large features first; public API changes must be discussed with the team first because the 1.x API is stable); links to internal docs (development, building, testing, triage, writing style); License (contributions licensed under MIT-0, confirmed via a bot checkbox on the first PR); Coding Style (CI enforces rustfmt and clang-format). **Details go into sub-documents under docs/; CONTRIBUTING itself only navigates.** |
| uv | <https://raw.githubusercontent.com/astral-sh/uv/main/CONTRIBUTING.md> | 387 lines. In order: Finding ways to help, Use of AI, Setup, Testing (including snapshot tests), Formatting, Linting, Crate structure, Profiling, Documentation, macOS code signing, Releases. |
| Bun | <https://raw.githubusercontent.com/oven-sh/bun/main/CONTRIBUTING.md> | 375 lines, almost all build environment setup and troubleshooting. |
| mpv | <https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/contribute.md> | In order: contact, Reporting issues ("report what you observe, not what you infer"), Sending patches (keep PR descriptions short; one PR does one thing), copyright and license statement, code style. |
| Raycast | <https://raw.githubusercontent.com/raycast/extensions/main/CONTRIBUTING.md> | 10 lines, all links to the docs site. |

---

## 2. Pattern comparison table

| Repository | Logo | Badges (count / kind) | Length | Quick start shape | Screenshot / GIF | Language switch | Example organization | Scaffolding |
|---|---|---|---|---|---|---|---|---|
| Vite | Centered, `<picture>` dark and light, 60px | 4: npm, node, CI, Discord | 66 lines (npm version 20 lines) | None, links straight to docs | None | None (docs site has translations) | monorepo packages table | `create-vite` |
| Slint | `#gh-*-mode-only`, two logos | 3: CI, REUSE, Discussions | about 240 lines | Hello World about 10 lines | Screenshot table | None | `examples/` plus screenshot table | C++/Rust template repositories |
| Tauri | Full splash image | 8 (on the many side) | 96 lines | One line `npm create tauri-app` | splash image | None | Separate plugins-workspace | `create-tauri-app` |
| uv | No logo; `<picture>` benchmark image | 3: PyPI, pyversions, Discord | 326 lines | 5 to 10 lines per feature | benchmark image | None | None | — |
| Bun | Centered 170px | 3: Discord, stars, speed | 446 lines | A set of commands | None | None | Docs guides | `bun init` |
| Deno | `align=right` floating | 5 social (reference style) | 116 lines | 3 lines of code plus 1 run line | None | None | Docs | `deno init` |
| Figma typings | None | 1: npm | 33 lines | Two steps: install plus tsconfig | None | None | None | Generated by the Figma desktop app |
| Obsidian api | None | 0 | 69 lines | None, links to template | None | None | sample-plugin template repository | GitHub Template |
| VS Code samples | `<h1 align=center>`, no image | 0 (yo code has 4) | 95 lines | clone, `npm i`, F5 | Each sample has its own GIF | None | Top-level `*-sample/` plus generated table | `yo code` |
| Raycast | Centered 128px plus large header image | 2 social, for-the-badge style | 31 lines | Links to docs | header image | None | The repository is the extension collection | `npm init raycast-extension` |
| mpv | 128px left-aligned | 0 | 222 lines | Build instructions | None | None | wiki | — |
| OBS template | None | 0 | 59 lines | Links to wiki | None | None | The template itself | GitHub Template |

**Conclusions**

1. **Standard hero section format**: a centered logo (60 to 170px), one row of 3 to 5 badges, a one-sentence tagline in bold or a blockquote, one row of navigation links (Docs • Examples • Changelog). Among the repositories surveyed, only Tauri has more than 5 badges, and it looks cluttered.
2. **Badge choice**: almost all have the package version (npm or PyPI) and CI. Next come the Node/runtime version, License, community (Discord). Download counts only appear on mature projects; a new package has few downloads, and showing them hurts more than it helps.
3. **README length**: type or SDK packages tend to be short (Figma 33 lines, Obsidian 69 lines, Vite 66 lines) and leave details to the docs site. The "too long" projects (Bun, uv) put a feature catalog into the README.
4. **Quick start**: at most three steps. Install in one line; minimal code of 3 to 10 lines; run or verify in one line. Deno and Slint both have this shape.
5. **Examples**: either a screenshot table (Slint) or a "name | one sentence | APIs used" table (VS Code samples, Tauri plugins), always in a separate `examples/` or a separate repository, with each example having its own README and screenshot.
6. **Scaffolding**: mature ecosystems all have `create-*` or `init`, or a GitHub Template repository. That lets the README quick start be a single command.
7. **Bilingual**: none of the surveyed repositories switch languages inside the README. A common practice in Chinese projects is `README.md` plus `README.zh-CN.md`, with a line of `English | 简体中文` links at the top. This comes from general knowledge of Chinese open source projects; no specific repository was fetched to verify it this time.
8. **CONTRIBUTING**: small projects (Slint, Raycast, mpv) keep it short, centered on PR rules, issue rules and the license statement, with build details pushed down into docs/. Only large projects (Vite, uv, Bun) spell out the full development environment. Recent repositories commonly have an **AI usage policy** section (Vite, uv, Tauri).

### Rendering differences between GitHub, npm and TypeDoc

| Markup | GitHub | npmjs.com | TypeDoc 0.28 (our docs site home page) |
|---|---|---|---|
| `<p align="center">` / `<div align="center">` | Supported | Mostly works, but not guaranteed (npm's sanitizer filters) | Outputs the HTML as is; supported |
| `<picture>` with `prefers-color-scheme` | Supported | **Not supported**: falls back to the inner `<img>`, or shows both (sources: [strip-gh-theme-links](https://cdn.jsdelivr.net/npm/strip-gh-theme-links@5.0.0/README.md), [Stefan Judis](https://www.stefanjudis.com/blog/how-to-define-dark-light-mode-images-in-github-markdown)) | Natively supported by the browser, but TypeDoc's theme switch is not the system preference, so they may not match |
| `#gh-dark-mode-only` | Deprecated | Shows both | Shows both |
| Relative image path `docs/assets/logo.png` | Supported | Relies on the `repository` field in package.json to resolve the path to the GitHub repository (sources: [Node-RED discussion](https://discourse.nodered.org/t/readme-information-missing-from-https-flows-nodered-org-node-page-for-a-new-node/80065), [gh-md-urls](https://npmjs.com/package/gh-md-urls)), resolved against the default branch | Copies the image into `media/`, provided the file exists locally |
| GitHub Alerts `> [!NOTE]` | Supported | Shown as a plain blockquote with a literal `[!NOTE]` | Supported from 0.26 |
| Badges (shields.io `<img>`) | Supported | Supported | Supported |

**Inference**: use an **absolute URL** for the logo, pointing to `raw.githubusercontent.com/.../main/...`, and use only a single `<img>`. Our gradient V works on both dark and light backgrounds, which conveniently removes the need for `<picture>`. Do not use Alerts, do not use `<picture>`. `<p align="center">` is safe to use; even if it fails on npm, it just degrades to left alignment without affecting content.

---

## 3. README rewrite recommendations

### 3.1 Principles

- The README only answers four questions: **what this is, what it looks like, how to install it, how to write a first plugin**. Everything else is linked out, to Getting started, the API reference, examples/, CHANGELOG, CONTRIBUTING.
- Target **60 to 90 lines of Markdown**, with the logo, badges, tagline and install command on the first screen.
- Maintainer content (Pages, CI, releases) is **moved out of the README entirely**.
- The `Versions` table keeps only three columns, "SDK | apiVersion | minimum player version", with one short value per cell. The list of APIs added in 1.1 moves to the CHANGELOG.
- Fix the outdated "Node 22.18" in `docs/getting-started.md`.

### 3.2 Section list (in order)

1. **Hero section** (centered)
   - logo: `<img src="https://raw.githubusercontent.com/vestrasys/vestra-player-sdk/main/.github/assets/logo.png" width="96" alt="Vestra Player">`
   - `<h1>Vestra Player SDK</h1>`
   - tagline: one bold line, for example **Write plugins for Vestra Player in TypeScript — with Slint UI, a test host and a one-command packager.**
   - Badge row (see 3.3).
   - Navigation row: `Documentation · Getting started · Examples · Changelog · 简体中文`
2. **One screenshot** (optional, but strongly recommended): an actual screenshot of a plugin window or panel in the player, about 720 wide, with an absolute URL. The user's criticism that it is "not fancy" is most directly solved by one image.
3. **Features**: 4 to 6 short bullet points, one line each, optionally with emoji, consistent with the Vite style:
   - A complete typed plugin API (`vestra/1`): commands, windows, panels, OSD, playback control, events
   - Native Slint UI components, with the same theme as the player
   - In-memory test host `@vestra-player/sdk/testing`, for unit tests without launching the player
   - `vestra-plugin` CLI: check, test, build, pack to produce a `.vxp`
   - The same API on macOS and Windows; each plugin runs in its own Node 26 process
4. **Install**: one line, `npm i -D @vestra-player/sdk`, with a line of small text below it, "Requires Node.js ≥ 26". Remove the current paragraph explaining Node and TypeScript and move it to Getting started.
5. **Quick start**: three code blocks, no more than 15 lines in total.
   - `manifest.json`: keep it as one line of JSON.
   - `main.ts`: keep the current 5 lines.
   - Run: `npx vestra-plugin test && npx vestra-plugin pack`. Once an `init` command exists, replace these three blocks with one line, `npx vestra-plugin init my-plugin`.
   - Closing link line: "→ Full walkthrough: Getting started".
6. **Examples**: a table, see section 5.
7. **CLI**: keep the current 4-line `text` block. It is already compact, in the style of Bun and yo code.
8. **Package entry points**: keep the current 3-row table. It can also be merged into item 9 as a small table under "Documentation".
9. **Compatibility**: a small three-column table, SDK | apiVersion | minimum player version, one short value per row, followed by one sentence with the semver promise.
10. **Contributing**: one line, linking to CONTRIBUTING.md.
11. **License**: one line, Apache-2.0.

Optional: adding "Related" at the end of the README, linking to the main player repository, the plugin store and the official plugin repository, helps conversion on npm.

### 3.3 Badges (5, shields.io recommended for a consistent style)

```html
<p align="center">
  <a href="https://www.npmjs.com/package/@vestra-player/sdk"><img src="https://img.shields.io/npm/v/@vestra-player/sdk?logo=npm&color=cb3837" alt="npm version"></a>
  <a href="https://github.com/vestrasys/vestra-player-sdk/actions/workflows/main.yml"><img src="https://img.shields.io/github/actions/workflow/status/vestrasys/vestra-player-sdk/main.yml?branch=main&label=CI&logo=github" alt="CI"></a>
  <a href="https://vestrasys.github.io/vestra-player-sdk/"><img src="https://img.shields.io/badge/docs-TypeDoc-6f42c1" alt="docs"></a>
  <a href="https://www.npmjs.com/package/@vestra-player/sdk"><img src="https://img.shields.io/node/v/@vestra-player/sdk?logo=node.js&logoColor=white" alt="node"></a>
  <a href="https://github.com/vestrasys/vestra-player-sdk/blob/main/LICENSE"><img src="https://img.shields.io/npm/l/@vestra-player/sdk" alt="license"></a>
</p>
```

- **The CI badge uses `main.yml` (name: Main), not `ci.yml`.** `ci.yml` does not trigger on the main branch (it covers pushes to non-main branches and PRs), so `?branch=main` would get no status or an outdated status. `main.yml` is the real state of the main branch and also includes the Pages deployment. The badge label uses `label=CI`, which reads more clearly to outsiders.
- The native GitHub badge also works: `https://github.com/vestrasys/vestra-player-sdk/actions/workflows/main.yml/badge.svg?branch=main`. Its text is the workflow name (i.e. "Main") and its style does not match shields, so shields is recommended.
- An optional 6th badge showing the plugin API version: `https://img.shields.io/badge/plugin%20API-vestra%2F1-ff2e88` (color taken from the logo's pink).
- **Not recommended for now**: downloads (a new package has small numbers, and showing them hurts), stars, Discord (there is no community channel yet; add it if a QQ group or Discord is opened later). A `types` badge is not needed either; the npm page already shows the TS icon.
- The package name has a scope; the shields path can use `@vestra-player/sdk` directly without escaping.

### 3.4 Logo handling

1. Scale `src/assets/brand/logo.png` down to **256×256** (displayed at 96px, covering 2x screens) and compress it with `oxipng` or `pngquant`; expected to be under 30 KB. Put it in the SDK repository at `.github/assets/logo.png`, or `docs/assets/logo.png`.
2. Reference it in the README with an **absolute raw URL**. That displays on GitHub, npm and TypeDoc alike, and the logo does not enter the npm package (`files` unchanged, package size does not grow).
3. Use only one `<img>`, not `<picture>`, for the reasons in the rendering differences table in section 2. If a monochrome logo is designed later, a `<picture>` can be added for GitHub, with the `<img>` fallback still pointing to the color version, so npm is not affected.
4. TypeDoc's `favicon` can be set to this logo (TypeDoc 0.28 supports the `"favicon"` option), so the docs site carries the same brand.
5. The screenshot from item 2 of section 3.2 also goes in `.github/assets/` and is referenced with an absolute URL.

### 3.5 Bilingual: recommend "English main README + full Chinese mirror"

- The product is Chinese-first, and plugin authors are likely Chinese users. But npm, the TypeDoc docs site, TSDoc comments, CLI output and manifest field names are all in English, and GitHub and npm search are global. The npm page can only show one `README.md`.
- Recommended approach:
  - `README.md` in English, `README.zh-CN.md` with the full text in Chinese. The two have exactly the same structure and code blocks; only the prose differs. Since the target is 60 to 90 lines, the sync cost is low.
  - Both navigation rows carry the language switch: `English | [简体中文](https://github.com/vestrasys/vestra-player-sdk/blob/main/README.zh-CN.md)`. **Use an absolute URL**, so the link does not 404 on the npm page.
  - Add `README.zh-CN.md` to `projectDocuments` in `typedoc.json`, so the docs site can find the Chinese version too.
  - Optional: later also make a Chinese version of `docs/getting-started.md`, `docs/getting-started.zh-CN.md`. What Chinese plugin authors really need is the getting started guide; the README only points the way.
- The alternative: `README.md` in Chinese, `README.en.md` in English. Choose this only if it is certain that "almost all developers are Chinese and npm exposure does not matter". The cost is that the npm and TypeDoc home pages become Chinese, mixed with the English API reference. **Not recommended.**

### 3.6 README draft skeleton (English version, about 75 lines)

```markdown
<p align="center">
  <img src="https://raw.githubusercontent.com/vestrasys/vestra-player-sdk/main/.github/assets/logo.png" width="96" alt="Vestra Player logo">
</p>
<h1 align="center">Vestra Player SDK</h1>
<p align="center"><b>Build plugins for Vestra Player in TypeScript — Slint UI, an in-memory test host and a one-command packager.</b></p>
<p align="center">(5 badges)</p>
<p align="center">
  <a href="https://vestrasys.github.io/vestra-player-sdk/">Docs</a> ·
  <a href="https://vestrasys.github.io/vestra-player-sdk/documents/Getting_started.html">Getting started</a> ·
  <a href="https://github.com/vestrasys/vestra-player-sdk/tree/main/examples">Examples</a> ·
  <a href="https://github.com/vestrasys/vestra-player-sdk/blob/main/CHANGELOG.md">Changelog</a> ·
  <a href="https://github.com/vestrasys/vestra-player-sdk/blob/main/README.zh-CN.md">简体中文</a>
</p>

<p align="center"><img src="https://raw.githubusercontent.com/.../.github/assets/screenshot.png" width="720" alt="A plugin window in Vestra Player"></p>

## Features
- (5 bullets)

## Install
npm i -D @vestra-player/sdk   # Node.js ≥ 26

## Quick start
manifest.json (1 line) / main.ts (5 lines) / npx vestra-plugin test && npx vestra-plugin pack
→ [Getting started](...) walks through windows, tests and packaging.

## Examples
(table, see §5)

## CLI
(4-line text block)

## Compatibility
| SDK | Plugin API | Player |
| 1.0 | vestra/1 | ≥ 0.1.0 |
| 1.1 – 1.2 | vestra/1 | ≥ 0.2.0 |
Minor releases add backwards-compatible API only; breaking changes bump `apiVersion` and the major version.

## Contributing · License
[CONTRIBUTING.md](...) · [Apache-2.0](LICENSE)
```

Two points to note:

- All links to files inside the repository (CHANGELOG, CONTRIBUTING, examples, zh-CN) must use absolute `https://github.com/vestrasys/vestra-player-sdk/blob/main/...` links so they are clickable on npm and TypeDoc. The current README already does this; keep it that way.
- `LICENSE` can use a relative path. npm handles it automatically, and TypeDoc can find the file too.

---

## 4. CONTRIBUTING recommendations

### 4.1 Split into a "contributors" file and a "maintainers" file

- **`CONTRIBUTING.md`** (for external contributors, target 50 to 70 lines, following Slint):
  1. A welcome, stating the areas where help is possible: bugs, docs, examples, type improvements.
  2. **Asking questions and reporting problems**: bugs use the issue template and must include the SDK version, player version, operating system and a minimal reproduction. Borrow mpv's principle: "report what you observe, not what you infer". Plugin development questions go to Discussions.
  3. **Discuss API changes first**: the SDK is the stable `vestra/1` API and must stay consistent with the player. Any addition to or change of the public API needs an issue for discussion first, because the player side has to implement it too. This is the most important rule for us (Slint has a similar clause).
  4. **Development environment**: Node 26, `npm ci`, and the 5 most used commands from the current set (check, lint, format:check, test, docs). The full list links to the maintainer docs.
  5. **PR rules**: one PR does one thing; changing the public API requires updating TSDoc (`npm run docs` treats warnings as errors), tests and the Unreleased section of the CHANGELOG; `ci-ok` in CI must pass; squash merge.
  6. **Commit and PR title style**: one imperative sentence, or `scope: summary`. Conventional Commits are only worth introducing if the changelog is to be generated by a tool; the CHANGELOG is currently handwritten, so they need not be introduced.
  7. **Slint files are read-only copies**: files under `slint/` are synced from the player; direct changes are not accepted; please file them against the player repository.
  8. **AI-assisted contribution policy** (optional; Vite, uv and Tauri all have one): AI may be used, but the submitter must understand and verify every line; unreviewed generated content is not accepted.
  9. **Code of conduct**: link to `CODE_OF_CONDUCT.md`, using Contributor Covenant 2.1; it does not exist yet and needs to be created.
  10. **License**: contributions are licensed under Apache-2.0 (inbound = outbound); one sentence is enough, no CLA needed.
- **`docs/maintaining.md`** (or `.github/MAINTAINING.md`): move the entire current CONTRIBUTING content there as is, including build and pack checks, TypeDoc rules, Pages, CI topology, releases, Trusted Publishing, syncing with the player. Put a line "Maintainers: see docs/maintaining.md" at the end of CONTRIBUTING.
  - Note: files under `docs/` that are not added to `projectDocuments` do not appear on the TypeDoc site, which is exactly what is wanted here.
- Also fill in `.github/`: `ISSUE_TEMPLATE/bug.yml` (with SDK version, player version, OS, reproduction steps), `ISSUE_TEMPLATE/config.yml` (directing questions to Discussions), `PULL_REQUEST_TEMPLATE.md` (3 checkboxes: tests, docs, CHANGELOG). Optionally `SECURITY.md`.

### 4.2 Chinese

CONTRIBUTING is likewise recommended to be English first. If the target contributors are mainly Chinese, a `CONTRIBUTING.zh-CN.md` can be added. The maintainer docs only need one language.

---

## 5. examples/ layout and presentation in the README

### 5.1 Directory (moved in from the player's `src/examples/`, plus one new minimal example)

```
examples/
  README.md                ← index table (same as the main README's table, or generated by a script)
  hello/                   ← new: minimal example, a command plus OSD, exactly the same as the README Quick start
    manifest.json  main.ts  tests/main.test.ts  README.md
  panel-menu/              ← ui.mount panel + playback control
    manifest.json  main.ts  panel.slint  tests/  README.md  screenshot.png
  bookmark/                ← ui.window + events + playback read/write: a complete small plugin
  core-ui/                 ← @vestra/ theme and core components, OSD
  markdown/                ← ui.markdown window, manifest display fields (Chinese name/description), icon
```

- Every example must contain `manifest.json`, `main.ts`, `tests/`, `README.md` (10 to 20 lines: what it does, which capabilities and APIs it uses, how to run it) and `screenshot.png` (except `hello`). This is the fixed structure promised by VS Code samples.
- Examples are **not put into the npm package** (`files` unchanged). Add a step `vestra-plugin check examples/* && vestra-plugin test examples/*` to `_test.yml` in CI, so the examples follow API evolution and do not rot. `vestra-plugin pack examples/*` can also be used as an extra smoke test along the way.
- **Single source**: after the move, decide whether the player's `src/examples/` references the SDK repository's version, or the two stay in sync with a consistency test (the same approach as the Slint copy). Recommended: examples live only in the SDK repository; if the player needs them for GUI testing, it takes them from the SDK repository.
- Example ids use `examples.<name>` consistently. `bookmark` is currently `community.bookmark` and needs to be aligned.

### 5.2 Presentation in the README

Use a mix of Tauri plugins-workspace and VS Code samples, in one table. Once the examples have screenshots, a row of 2 to 3 thumbnails can be placed above the table (the Slint approach):

```markdown
## Examples

| Example | Shows | Capabilities |
| --- | --- | --- |
| [hello](…/examples/hello) | A command and an on-screen message — the Quick start, runnable | `commands`, `ui.osd` |
| [panel-menu](…/examples/panel-menu) | A Slint panel mounted in the player, driving playback | `ui.mount`, `playback.control` |
| [bookmark](…/examples/bookmark) | A window that saves and jumps to bookmarks | `ui.window`, `events`, `playback.read` |
| [core-ui](…/examples/core-ui) | The `@vestra/` theme and widgets | `ui.mount`, `ui.osd` |
| [markdown](…/examples/markdown) | A Markdown document window, display fields and icon in the manifest | `ui.window` |

Run one: `cd examples/hello && npm i && npx vestra-plugin test`
```

### 5.3 Later: scaffolding (outside the scope of this rewrite, but it decides the final shape of Quick start)

- Short term: make `examples/hello` a template that can be copied directly, or create a separate GitHub Template repository `vestra-plugin-template` (the Obsidian and OBS model), with "Use this template" in the README.
- Medium term: add `vestra-plugin init [dir] [--template hello|window|panel]`, which copies from `examples/` and rewrites the id, or publish `create-vestra-plugin` to support `npm create vestra-plugin@latest` (the Vite, Tauri and Raycast model). At that point the README Quick start can shrink to:

```sh
npm create vestra-plugin@latest my-plugin
cd my-plugin && npm test && npm run pack
```

---

## Appendix: to-do list (by priority)

1. Handle the logo: scale to 256px and compress, put it at `.github/assets/logo.png`, reference it in the README with an absolute raw URL.
2. Rewrite README.md per sections 3.2 and 3.6 (60 to 90 lines). Write README.zh-CN.md alongside it and add it to TypeDoc's projectDocuments.
3. Take a screenshot of a plugin inside the player and put it in `.github/assets/`.
4. Move the current CONTRIBUTING to `docs/maintaining.md`, write a new contributor-facing CONTRIBUTING, and add CODE_OF_CONDUCT, issue templates and a PR template.
5. Create `examples/`: move the 4 examples in from the player, add `hello`, and check and test these examples in CI.
6. Fix "Node 22.18" in `docs/getting-started.md`.
7. Optional: TypeDoc `favicon`; later build `vestra-plugin init` or `create-vestra-plugin`.
