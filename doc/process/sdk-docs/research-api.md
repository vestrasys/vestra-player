# Research: rewriting the `@vestra-player/sdk` API documentation

Date: 2026-10-22. Scope: read-only research, no code changes.
Subject: the SDK repository clone `scratchpad/sdkrepo` (v1.2.0, TypeDoc 0.28.20, site https://vestrasys.github.io/vestra-player-sdk/), and the long-form references in the player repository: `doc/plugins/reference.md` (1658 lines), `tutorial.md` (357 lines), `examples.md` (939 lines, 34 sections).

---

## 0. Conclusions first

1. **The problem is not only that the comments are short; the conceptual material is in the wrong place.** The background knowledge (update passes, name conversion, handles, the global bridge for panels, the full-screen sidebar, what the error codes actually mean) is all written in the Chinese `reference.md` / `tutorial.md` / `examples.md` in the player repository, and the SDK site links to none of it. The SDK site has only the English TSDoc plus one getting-started page, and the TSDoc itself is written in a "specification" style: short, dense, and assuming the reader already knows the concepts.
2. What good documentation has in common (MATLAB, man, Rust, MDN, Stripe, Qt, VS Code, OpenCV, Django, Node): **concept pages and reference pages are written separately and link to each other.** Every symbol's reference entry follows a fixed section order (summary, description, parameters, return value, errors, examples, see also, version), and the examples are complete, runnable, and checked by machine.
3. Recommended approach:
   - Split the site by Diátaxis into four kinds: tutorials, concepts, how-to guides, API reference.
   - Move the long Chinese documents from the player repository into the SDK repository, where they become the **single source of truth**; `reference.md` is reduced to a redirect page.
   - Complete the TSDoc with a uniform template: `@remarks`, `@throws` (our error codes), a custom `@capability`, `@example`, `@see`, `@since`.
   - Write examples as real `.ts` files under `examples/`, pull them into the docs with `{@includeCode}`, check them with `tsc` in CI, and actually run them once against the `startPlugin` test host (the equivalent of a doctest).
   - Language: **Chinese as the primary language**, set `lang: "zh"`; keep an English introduction in the README and on the npm entry.
4. Effort: about 30 "module entry" types and about 140 methods need a full rewrite to the template; about 60 properties need units and defaults; in addition about 9 concept pages and about 20 how-to guides (most can be adapted from `examples.md`).

---

## 1. Current state

### 1.1 Numbers

| Metric | Value | Notes |
| --- | --- | --- |
| Exported types in `types.ts` | 103 | About 390 members, of which about 137 are methods |
| Single-line `/** … */` comments | 434 (types.ts), 172 (testing.ts) | Most methods have only one sentence |
| `@example` | 79 (types.ts 44, testing.ts 28) | Examples are **not type-checked**: `tsconfig.check.json` only covers `src/` and `test/`, and examples often use undefined variables (`store`, `load`, `stopPolling`, `askUser`, `chain`, `settings`) |
| `@throws` / `@returns` / `@param` | 3 / 7 / 23 | Errors are mostly written in the body text, e.g. "`invalid` for …" |
| `@remarks`, `@see`, `@since`, `@defaultValue`, `@category` | **all 0** | No version information and no grouping; the sidebar lists 103 types flat in source order |
| Hand-written pages on the site | 1 (`docs/getting-started.md`, 239 lines) | `projectDocuments` contains only this one |
| TypeDoc validation | `notDocumented` and `invalidLink` on, warnings treated as errors | A good base: every export has a comment and links do not break |

Conclusion: **coverage is not the problem; depth and structure are.** Every symbol has a comment, but almost all of them only say "what it is", not "why, where it sits in the whole, how to use it, how it fails".

### 1.2 Five typical weak spots (with the original text)

#### ① `Windows.create` / `Window`: does not answer "what exactly is a tool window"

```ts
/** `plugin.windows`: tool windows. Needs `ui.window`. */
export interface Windows {
  /**
   * A tool window from a `.slint` file in the package. Nothing opens until `show()`.
   ...
```

```ts
/**
 * A tool window, from {@link Windows.create}. Disposing it closes the
 * window.
 * @example
 * const info = plugin.windows.create("ui/info.slint");
 * info.on("refresh", () => load());
 * info.onClose(() => stopPolling());
```

What is missing:

- **Concept**: the comment does not say that it is an independent, system-level top-level window. From the player source `src/host/src/ui.rs::open`, it is the Slint instance's own native window: it has its own title bar, is non-modal, and is not attached to the main window; size and title come from the Slint `Window` component; before showing it, the player applies the theme colors and the system font. The comment also does not say how it differs from panels, settings pages and dialogs, or when to choose which.
- **Lifecycle**: the sequence `create → on → show → hide/close → user closes → dispose` is never explained as a whole; "calling `show()` after closing gives a brand-new instance" is scattered across the comments of three members.
- **Standard close keys**: the user pressing ⌘W, or Ctrl+W / Ctrl+F4 on Windows, is the same as clicking close in the title bar and triggers `onClose`. This is only written in `reference.md` §6.2.
- **Errors**: which codes `show()` can reject with is not documented at all. Inferred from the host code (the `"ui.open"` branch in `main.rs`), at least: `unavailable` (headless mode), `capability`, file outside the package or missing (errors from `plugins::package_file`), Slint compile failure. The last one is mapped by `From<anyhow::Error>` to **`host`**, and this code is not in the `PluginError` error table.
- A duplicate id throws **a plain `Error` without a `code`** (`ui.ts`: `throw new Error(\`window ${id} already exists…\`)`), which contradicts `PluginError`'s promise that "every failure has a stable code".
- **Example**: only the TS half, with no matching `.slint` file; `load` and `stopPolling` are undefined, so the example does not run when copied.

#### ② `Player` control methods: one sentence, no ranges, errors or conditional semantics

```ts
/** Pause or resume. */
setPaused(paused: boolean, options?: Conditional): Promise<void>;
/** Jump to `seconds`. */
seek(seconds: number, options?: Conditional): Promise<void>;
/** Set the playback speed. */
setRate(rate: number, options?: Conditional): Promise<void>;
```

Not documented: the range `setRate` accepts, what `seek` does beyond the duration, whether a non-seekable source gives `unsupported`, whether no media gives `unavailable`. The conditional-commit semantics of `stale` are mentioned only once, at the top of the `Player` interface. The long reference §5.2 likewise only says "seek" and "speed". These methods are exactly what MATLAB's Input Arguments and man's ERRORS cover best.

#### ③ `Settings.page` / `Panels`: the "one component and one global" bridge concept is never explained

```ts
/**
 * Add a page (the file exports one component and one global, like a
 * panel). ...
 * @example
 * page.bind(() => ({ resume: store.value.resume }));
 * page.on("resumeChanged", (on: boolean) => store.save({ ...store.value, resume: on }));
```

- Why is a window "one component" while panels and settings pages are "component + global"? What role does the global play here, and where should the properties go? These are the biggest obstacles for Slint beginners. tutorial.md has a section "there are only two directions between the UI and the plugin", but it is not visible on the SDK site.
- `store` in the example is an undefined variable that silently relies on `plugin.storage.json(...)`. The example gives no `.slint` file and does not handle `page.ready` (which rejects with `ui-prepare` when the file fails to compile).

#### ④ `Packages`: a process-style API without a description of the process, and field values without enumerations

```ts
/** Validate a package directory (`plugins.manage`). */
validate(directory: string): Promise<PackageCandidate>;
/** Restore a plugin's previous version (`plugins.manage`). */
rollback(id: string): Promise<PackageChange>;
...
/** The checks the player ran. */
readonly checks: string[];
/** The trial's status as the player reports it. */
readonly status: string;
```

- The docs do not describe the state machine "validate → trial load (trial) or install → `change.done` result", nor what validate actually checks (manifest, `minPlayerVersion`, which gives `versionMismatch`).
- `checks` and `status` are open `string`s; their possible values are not listed.
- The author of a "plugin store" plugin has to read `store-design.md` in the player repository to get it right.

#### ⑤ The error model and facts drift across places

- The `PluginError` error table has no `versionMismatch`, but `reference.md` §13 has it, and the host `main.rs` does return it. The host also returns `host` (Slint compile failures and others), `ui-prepare` and `ui-value`, none of which are in the SDK table.
- `docs/getting-started.md` says "Node.js process (**Node 22.18** or newer)", while `package.json` has `"node": ">=26"`, and `reference.md` also says "built-in Node 26". The example `package.json` still has `"@vestra-player/sdk": "^1.0.1"`, while the current version is 1.2.0.
- The same specification is written in two places (TSDoc in English, `reference.md` in Chinese), and `reference.md` itself admits at the top "where this document disagrees with the code, the code wins". This is a **structural source of drift**.

> Places that are already written well and can seed the template: `Disposable` (lists what each kind of handle does when released), `PluginError` (error code table), `Scope` (a complete example with child scopes), `CommandOptions.bindings` (syntax, platform differences and conflict rules all covered), `Ui.markdown` (limits, caching and version requirements all covered). The problem is that these are the exception, not the norm.

---

## 2. How good documentation does it

### 2.1 MATLAB function reference

Source: https://www.mathworks.com/help/matlab/ref/double.interp1.html

- Fixed section order: **Syntax → Description → Examples → Input Arguments → Output Arguments → More About / Tips / Algorithms → References → Extended Capabilities → Version History → See Also**.
- **Syntax and Description correspond one to one**: each calling form (`vq = interp1(x,v,xq)`, `vq = interp1(v,xq)`, `pp = interp1(x,v,method,"pp")`) has its own paragraph of description, so readers look things up by "I want to call it like this".
- **Examples are titled as tasks**, for example "Interpolation of Coarsely Sampled Sine Function", "Designate Constant Value for All Queries Outside the Domain of x". Each example is self-contained and can be run directly.
- **Each Input Argument has fixed fields**: name, shape or class, description, constraints, data types, default, example value; collapsible.
- **Extended Capabilities** lists platform or environment differences separately (for example `"pchip"` is not supported on GPU). For us, that means the differences between macOS and Windows, and under headless mode and software rendering.
- **Version History** lists "Introduced in R20xx" and later behavior changes by version.

Takeaways for us:

- Put errors and platform differences in their own sections, not buried in body paragraphs.
- Title examples as "tasks".
- State the range, unit and default of each parameter.

### 2.2 Linux man-pages conventions

Source: https://man7.org/linux/man-pages/man7/man-pages.7.html

- Section order: NAME, LIBRARY, SYNOPSIS, DESCRIPTION, RETURN VALUE, **ERRORS**, ENVIRONMENT, FILES, ATTRIBUTES, VERSIONS, STANDARDS, HISTORY, NOTES, **CAVEATS**, BUGS, **EXAMPLES**, SEE ALSO.
- **Rule for ERRORS**: list every possible errno value and its cause. When the same error code corresponds to different conditions, split it into several entries ("separate list entries (with duplicate error names) for each of the conditions"), sorted alphabetically. This maps directly onto our error model, which branches on `code`.
- **Rule for EXAMPLES**: example programs should be short, complete, compile without warnings, and include error checking.
- **CAVEATS / BUGS**: common misuse and known limitations get their own sections. For us, that is traps like "callbacks must be bound before the first `show()`".
- HISTORY / VERSIONS: record when something appeared and when it changed significantly.

### 2.3 Rust std / rustdoc

Sources: https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html , https://doc.rust-lang.org/std/fs/struct.File.html

- Recommended template: one sentence on what it is; a more detailed explanation; at least one example that can be copied and tried; advanced notes if needed.
- The **summary line** (everything before the first blank line) is reused in search results and module overviews, so it must stand on its own as a sentence.
- Conventional heading sections: `# Errors`, `# Panics`, `# Safety`, `# Examples`. The Errors section of `File::open` says "This function will return an error if `path` does not already exist. Other errors may also be returned according to `OpenOptions::open`."
- **Type-level documentation explains the concept first.** Before the method list, the `File` page explains "closed automatically when it goes out of scope, errors on close are ignored, use `sync_all` if you need the error", plus "not buffered, wrap it in a `BufReader`" and the Windows-specific behavior. This is exactly the paragraph our `Window` lacks.
- **Examples are tests**: rustdoc compiles and runs the code blocks in the documentation, so examples do not rot.
- Crate and module-level documentation (`//!`) first explains, in less technical language, "where this module sits in the ecosystem".

### 2.4 MDN Web API

Sources: https://developer.mozilla.org/en-US/docs/Web/API/Window/fetch , https://developer.mozilla.org/en-US/docs/Web/API/Window

- Sections of a method page: Syntax → Parameters → Return value → **Exceptions** → Examples → Specifications → Browser compatibility → See also.
- **Exceptions are formatted as "exception type + list of trigger conditions"**, for example under `TypeError` it lists "The requested URL is invalid", "…includes credentials" and so on, one per item. This can be copied directly as "error code + conditions".
- An interface page (`Window`) starts with a two-sentence definition of the concept: "The `Window` interface represents a window containing a DOM document…". Then the inheritance, and lists of properties, methods and events grouped by category, one-line summary each, with Read only / Deprecated / Experimental badges.
- **Reference pages and concept guides link to each other**: the `fetch()` page links to the long guide "Using Fetch"; every API family also has an "X API" overview page that explains the concepts and usage and lists the interfaces. Examples have both a live runnable version and GitHub source.

### 2.5 Stripe API reference

Source: https://docs.stripe.com/api/refunds/create

- Two-column layout: description and parameters on the left, request code and a **complete example response object** on the right.
- It starts with the business meaning, not the syntax: "You can optionally refund only part of a charge … Once entirely refunded, a charge can't be refunded again. This method will raise an error when…".
- **Prerequisites**: it tells the reader explicitly which objects must be created before this code runs.
- Parameters follow one format: "name (type, required/optional) + description + optional enum values"; nested parameters can be expanded.
- The Returns section says both what is returned on success and when it errors, and links to a shared errors page.

Takeaways for us:

- The first paragraph of a method comment says "what this operation means in business terms, and when it fails".
- Examples state their prerequisites, for example "needs `ui.window` declared in the manifest, and `ui/info.slint` in the package".

### 2.6 VS Code Extension API

Sources: https://code.visualstudio.com/api/extension-capabilities/overview , https://code.visualstudio.com/api/extension-guides/webview , `vscode.d.ts` (https://github.com/microsoft/vscode/blob/main/src/vscode-dts/vscode.d.ts)

- **Key fact: `vscode.d.ts` itself is also brief.** For example, the comment on `createWebviewPanel` is only "Create and show a new webview panel." plus `@param` and `@returns`. All its depth is in the **concept guides**: the Webview guide starts with a one-sentence definition ("Think of a webview as an `iframe` within VS Code that your extension controls…"), then "Should I use a webview?", then lifecycle, message passing, security, persistence and accessibility in turn. The guide links to runnable samples in `vscode-extension-samples`.
- Site structure: Get Started (tutorials) → Extension Capabilities (what you can do, concepts) → Extension Guides (how to do each capability) → UX Guidelines → References (API, Contribution Points, Activation Events).
- The most useful point for us: we are the same kind of "host + plugin API" as VS Code. **The reference can be compact, provided there are concept guides and they link to each other.** Our current situation is that concept guides exist, but they are not on the site and are not linked.

### 2.7 Qt

Source: https://doc.qt.io/qt-6/qwidget.html

- Structure of a class page: header table (Header, Module, Inherits, Inherited By) → list of properties and functions → **a long Detailed Description** with concept subsections (Top-Level and Child Widgets, Composite Widgets, Size Hints and Size Policies, Events…) → member details.
- The concepts are written on the class page itself. For example, "A widget without a parent widget is always an independent window (top-level widget)… Non-window widgets are child widgets, displayed within their parent widgets." This is exactly the kind of answer we want for "what exactly is a window".
- Every member has "See also" and "This function was introduced in Qt 6.x".

### 2.8 OpenCV

Sources: tutorial source https://github.com/opencv/opencv/blob/4.x/doc/tutorials/imgproc/imgtrans/canny_detector/canny_detector.markdown , Python tutorial https://github.com/opencv/opencv/blob/4.x/doc/py_tutorials/py_imgproc/py_canny/py_canny.markdown , reference comments https://github.com/opencv/opencv/blob/4.x/modules/imgproc/include/opencv2/imgproc.hpp . docs.opencv.org sits behind Cloudflare and fetching it returns 403, so the sources used to generate the site are cited here.

- Fixed tutorial structure: **Goal → Theory → Code → Explanation → Result**, chained into a learning path with `@prev_tutorial` / `@next_tutorial`.
- **Code in tutorials is not hand-written**: `@include samples/cpp/tutorial_code/ImgTrans/CannyDetector_Demo.cpp` pulls in a whole file, `@snippet … variables` pulls in a named fragment of a file, and each part is explained in turn; `@add_toggle_cpp/java/python` provides language switching. The samples are real programs that get compiled, so the code in the documentation does not drift from the API.
- Reference entry (`Canny`): one sentence of `@brief`, then the algorithm description with external links, and each `@param`; the class header also has `@example samples/cpp/edge.cpp … Check the corresponding tutorial`, linking the reference back to the tutorial.
- **TypeDoc 0.28's `{@includeCode file.ts#region}` is the equivalent of this mechanism**, see §4.3.

### 2.9 Django and Diátaxis

Sources: https://docs.djangoproject.com/en/stable/ , https://diataxis.fr/

- Django's four kinds of documentation:
  - **Tutorials**: "take you by the hand through a series of steps…";
  - **Topic guides**: "discuss key topics and concepts at a fairly high level and provide useful background information and explanation";
  - **Reference guides**: "technical reference for APIs… assume that you have a basic understanding of key concepts";
  - **How-to guides**: "recipes… guide you through the steps involved in addressing key problems".
- The two axes of Diátaxis are "action / cognition" and "acquisition / application". Its requirements for reference documentation: **describe** rather than instruct, be **austere**, be **consistent in structure**, and **mirror the structure of the product**.
- What this means for us: the user's criticism ("no background", "don't know what a window is") is a missing **Explanation** layer, and it cannot be solved only by stuffing long text into reference comments. The reference holds "the necessary background (a few sentences) + a link to the concept page"; the full explanation goes on the concept page.

### 2.10 Node.js API docs

Source: https://nodejs.org/api/fs.html

- Every API is marked with a **Stability** ("Stability: 2 - Stable") and has **"Added in: vX" plus a version history table**, for example "v21.1.0 The `flush` option is now supported."
- Parameters follow one format, "`` `position` <integer> | <null> … **Default:** `null` ``", and return values are written as "Returns: <Promise> Fulfills upon success with…".
- Examples are given in both ESM and CJS. Examples include `try/finally` to release resources, showing the correct usage rather than only the minimal one.

### 2.11 What TypeDoc / TSDoc provide

Sources: https://typedoc.org/documents/Tags.html , https://typedoc.org/documents/Options.Output.html , and checked against the local `node_modules/typedoc` (0.28.20) source.

| Need | TypeDoc 0.28 support | Notes |
| --- | --- | --- |
| Summary and detailed description | First paragraph is the summary; `@remarks`; `@summary` (for list summaries) | |
| Parameters and return value | `@param`, `@returns`, `@typeParam`, `@defaultValue` | Inline object parameters can be written as `@param options.id -` |
| Errors | `@throws` | One `@throws` per code, the equivalent of man's ERRORS |
| Examples | `@example Title` (the title is shown) | Several allowed |
| Including external code | **`{@includeCode ../examples/windows.ts#basic}`** (cut by `#region` name, several regions separated by commas), `{@includeCode file:10-20}` (by lines), `{@include doc.md}` | Checked in the local source: paths are relative to the file containing the comment; cutting by region supports extensions such as `ts`, `css`, `md`, `c`, `cs`, **but not `.slint`**; `.slint` can only be included by line numbers or as a whole file |
| Cross-links | `{@link X}`, `{@linkcode}`, `@see` | `validation.invalidLink` is already on |
| Versions | `@since` | Built in |
| Grouping | `@category` / `@categoryDescription`, `@group` / `@groupDescription`, `navigation.includeCategories` | Solves the problem of 103 types listed flat |
| Custom tags | `blockTags` option plus a `tsdoc.json` declaration | For `@capability` and `@player` (required player version) |
| Hand-written pages | `projectDocuments`; frontmatter supports `group`, `category`, `children` | Tutorials, concepts and how-to guides can all live on the same site |
| Localization | `lang: "zh"`, **built-in Chinese UI translation** | Checked locally: `dist/browser-locales/zh.js` exists |
| Search | `searchInComments`, `searchInDocuments` | When on, Chinese body text is searchable too |
| Links to external types | `typedoc-plugin-mdn-links` 5.1.1 (peer `0.27.x \|\| 0.28.x`) | `AbortSignal`, `Error`, `Promise` link to MDN automatically |
| Themes | Default theme; `typedoc-github-theme` 0.4.0 (`~0.28.0`); `typedoc-material-theme` 1.4.1 (`^0.28.x`) | Another route: `typedoc-plugin-markdown` 4.14 plus `typedoc-vitepress-theme` 1.1.4, generated by VitePress (built-in i18n and a better experience for hand-written pages) |
| Coverage | `typedoc-plugin-coverage` 4.0.3 | Generates a documentation coverage badge |

---

## 3. Our documentation standard

### 3.1 Site structure (Diátaxis)

```
vestra-player-sdk documentation site
├── Getting started (Tutorial)
│   ├── 1. First plugin: a command + an on-screen message (10 minutes)
│   ├── 2. Add a tool window (Slint ⇄ TS in both directions)
│   ├── 3. Testing: use TestHost without starting the player
│   └── 4. Check, run, package, publish to the store
├── Concepts (Explanation / Topic guides)
│   ├── How a plugin runs: process, activate, capabilities, trust model, platforms
│   ├── Handles and scopes: Disposable, Scope, child scopes, cleanup on deactivation
│   ├── Update passes and bindings: bind / set / Bindable, when values are re-evaluated, batched sends
│   ├── UI overview: tool window / panel (bottom bar, sidebar, background layer) / settings page / dialog / OSD — how to choose
│   ├── Slint bridge: properties and callbacks, in / in-out, name conversion, component + global, theme and widget library
│   ├── Commands, menus and shortcuts: paths, order / group, binding syntax, conflict priority, user rebinding
│   ├── Player state: PlayerState, sourceId, conditional commits (ifCurrent / stale), events and the events capability
│   ├── Media processing: filter chains, display layers, render options, generated streams
│   ├── Error model: list of codes, onError, un-awaited promises, timeouts
│   └── Settings and data: division of roles between storage, secrets, dataDirectory and settings pages
├── How-to guides (mostly adapted from the 34 sections of examples.md)
│   ├── Resize the window to the video size / keep it on top
│   ├── Sidebar playlist: list model, selection, auto-advance, context menu
│   ├── Add a settings page to the Options window
│   ├── Save an API key in the system keychain
│   ├── Resolve a network address (Resolver) and wait until ready
│   ├── Take and export screenshots, show progress, support cancel
│   ├── Validate, trial-load and install a plugin package
│   └── …
├── API reference (TypeDoc, grouped by @category)
│   ├── Core: Plugin, Scope, Disposable, PluginError
│   ├── Commands and shortcuts: Commands, Command, CommandOptions, Bindings
│   ├── UI: Windows, Window, View, Panels, Panel, Settings, Dialogs, Osd, Ui, Drops, Appearance, MainWindow
│   ├── Playback: Player, Media, Scrub, Stream, Timeline, Host
│   ├── Processing: VideoFilter, AudioFilter, VideoView, RenderOptions, Subtitles, MediaCapture
│   ├── System: Storage, Secrets, System, Clipboard, Power, Tools, App
│   ├── Plugin management: Packages, Resolvers
│   └── Testing: @vestra-player/sdk/testing
└── Appendices: error code table, capability table, platform differences table, version history (CHANGELOG)
```

A few rules:

- **Single source of truth**: the body text of concepts and how-to guides moves to `docs/` in the SDK repository. The player repository's `doc/plugins/reference.md` keeps only implementation-side notes and "change notes"; the public specification always links to the SDK site.
- **Two-way links**:
  - every reference entry has at least one `@see` pointing to the matching concept page;
  - every concept page lists "APIs covered on this page", using `{@link}`;
  - the code in every how-to guide comes from `examples/` and shares the same files with the examples in the reference.
- Error codes, capabilities and platform differences each have **one master table**; a reference entry only says "which codes this method produces, and under what conditions", and links to the master table.

### 3.2 Comment template for each symbol

Applies to: module entries (`Windows`, `Player`…) and all methods. Plain data fields only need a summary, plus unit, range and default.

````ts
/**
 * â¨Summary: one sentence, starting with a verb, saying what it does. Reused in lists and search, so it must stand on its own.â©
 *
 * @remarks
 * â¨Background: 2–6 sentences. Where it sits in the whole; lifecycle or state changes; how it differs from similar APIs (when to use X instead);
 *   constraints that hold throughout (for example "callbacks must be bound before the first show()"). For long background, write only the conclusion and link to the concept page for details.â©
 *
 * **Platform**: â¨Differences under macOS / Windows / headless mode / software rendering. Omit when there are none.â©
 *
 * @capability â¨Manifest capability, for example ui.window. Write "none" when no capability is needed.â©
 * @player â¨Minimum required player version, only when higher than the vestra/1 baselineâ©
 *
 * @param name - â¨Meaning; unit; range; default; format. Whether it is optional is expressed by the type and not repeated hereâ©
 * @param options.id - â¨Document inline object parameters field by fieldâ©
 * @returns â¨When it resolves and with what value. "What has taken effect when it resolves" must be stated clearlyâ©
 *
 * @throws `invalid` â¨Synchronous throw / asynchronous rejection, keep the two apartâ©â¨conditionâ©
 * @throws `unavailable` â¨conditionâ©
 *   â¨One entry per code; when the same code has different conditions, split it into several entries (the man-pages rule); order by "most likely for the caller to hit"â©
 *
 * @example â¨Task-style title, for example "Open an info window from the menu"â©
 * {@includeCode ../../examples/snippets/windows.ts#info-window}
 *
 * @see {@link RelatedSymbol}, [Concept: UI overview](../documents/ui-overview.html)
 * @since 1.0.0
 * @category UI
 */
````

Writing rules:

- **Summary**: at most one line, does not start with "This" or "A", does not repeat the type name.
- **`@remarks`**: must answer "when to use it, and when not to".
- **`@throws`**: use only codes from the `PluginError` master error table; state clearly whether it is a synchronous `throw` or a Promise rejection. Codes outside the table (`host`, `ui-prepare`, `versionMismatch`) are added to the table first.
- **`@example`**:
  - must compile: include it from `examples/` with `{@includeCode}`, or extract inline code blocks in CI and check them with `tsc`;
  - must be self-contained, with no undefined variables;
  - for UI, also give the `.slint` file, included by line numbers with `{@includeCode ../../examples/ui/info.slint:1-12}`;
  - the first example shows the most common usage; the second shows a trap or an advanced usage.
- **Division between methods and modules**: module entries (`Windows`, `Player`) carry the concept paragraph and an overall example; methods carry their own parameters, errors and small examples, and do not repeat the concept paragraph.
- **Glossary** (for the Chinese docs):
  - 句柄 (handle), 作用域 (scope), 能力 (capability), 更新轮 (update pass), 绑定 (binding);
  - 工具窗口 (tool window), 面板 (panel), 桥接 global (bridge global), 条件提交 (conditional), 来源 (source).
  - Code identifiers and error codes stay in English as they are.

### 3.3 Full example: rewriting `Windows` / `Windows.create` / `Window` with the template

The facts below are based on: `WindowHandle` in SDK `src/plugin/ui.ts`, `types.ts`, `reference.md` §6.2 and §6.4, and the player's `src/host/src/ui.rs::open` and the `"ui.open"` branch in `main.rs`. Entries marked [to verify] need confirmation from the implementer, or a code fix first.

````ts
/**
 * Tool windows: independent windows that a plugin defines in a `.slint` file and the player opens.
 *
 * @remarks
 * **What a tool window is.** It is an independent top-level system window: it has its own title bar, can be moved and resized,
 * is non-modal (while it is open the main window can still be used and keeps playing), and is not embedded in the main window. The window's content, initial size and title
 * are all determined by the `inherits Window` component in the `.slint` file (`title`, `preferred-width` and so on).
 * Before showing the window, the player applies the current color scheme ({@link Appearance}) and the system font, so the plugin does not need to handle themes.
 *
 * **Choosing a UI form.**
 * - For a standalone tool that you "open → use → close" (picture adjustment, info panel, editor), use a tool window;
 * - For content that stays in the player and follows the main window layout (playlist, progress bar, cover art), use a
 *   {@link Panels.mount | panel};
 * - For preferences, use a {@link Settings.page | settings page};
 * - To ask just "are you sure?", use {@link Dialogs.confirm}.
 * For a comparison see [Concept: UI overview](../documents/ui-overview.html).
 *
 * **Lifecycle.** `create()` only creates a local handle and opens nothing. After that:
 * 1. bind callbacks with {@link View.on} and declare properties with {@link View.bind};
 * 2. the first {@link Window.show} opens the window with the currently bound properties as initial values, without flashing the defaults;
 * 3. {@link Window.hide} hides the window but keeps its state;
 * 4. {@link Window.close}, the user clicking close in the title bar, or the standard close keys (macOS ⌘W; Windows Ctrl+W or Ctrl+F4)
 *    all destroy the window instance. A later `show()` opens a brand-new instance with the latest properties. {@link Window.onClose}
 *    is called only when the user closes it.
 *
 * The handle belongs to the {@link Scope} that created it. When the scope is released or the plugin is deactivated, the window closes automatically; the plugin does not need to clean up.
 *
 * **Platform**: macOS and Windows behave the same. In headless mode, `show()` fails with `unavailable`.
 *
 * @capability ui.window
 * @see [Concept: Slint bridge](../documents/slint-bridge.html), {@link Window}, {@link View}
 * @since 1.0.0
 * @category UI
 */
export interface Windows {
  /**
   * Create a tool window handle for a `.slint` file in the package; no window opens until {@link Window.show} is called.
   *
   * @remarks
   * The file must **export exactly one component**, usually written as `export component X inherits Window`. Helper components
   * without `export` are unlimited. This differs from panels and settings pages, which require exporting
   * "one component plus one global".
   *
   * The window only forwards **callbacks whose names were bound by the first `show()`**. This is because the player registers
   * the callback names once when it opens the window. So bind all callbacks with {@link View.on} before `show()`. Binding a new name afterwards
   * throws synchronously; the same name can be rebound. Properties have no such restriction.
   *
   * To open several windows from the same file (for example one info window per file), distinguish them with `options.id`.
   *
   * @param file - Path relative to the plugin package directory, for example `"ui/picture.slint"`. Must not go outside the package directory.
   * @param options.id - Window id, unique within this plugin. Defaults to the file name without extension
   *   (`ui/picture.slint` → `picture`). The id can be reused after the window is disposed.
   * @returns A {@link Window} handle that belongs to the current scope.
   *
   * @throws (synchronous) This plugin already has a window with the same id.
   *   [To verify / to fix: currently throws a plain Error without `code`; should be `invalid`]
   *
   * @example Open an info window from the menu
   * {@includeCode ../../examples/snippets/windows.ts#info-window}
   *
   * The matching `ui/info.slint`:
   * {@includeCode ../../examples/snippets/ui/info.slint}
   *
   * @example Open several windows from the same file
   * ```ts
   * const left = plugin.windows.create("ui/compare.slint", { id: "compare-left" });
   * const right = plugin.windows.create("ui/compare.slint", { id: "compare-right" });
   * ```
   *
   * @see {@link Panels.mount}, {@link Settings.page}
   * @since 1.0.0
   */
  create(file: string, options?: { id?: string }): Window;
}

/**
 * A handle to one tool window, from {@link Windows.create}.
 *
 * @remarks
 * The rules for properties and callbacks (the difference between `bind` and `set`, name conversion, retry on failed writes) are inherited from {@link View}.
 * There are only two window-specific rules:
 * - callbacks must be bound before the first `show()`;
 * - the user can close the window themselves.
 *
 * After the user closes it, `opened` becomes `false` and the handle remains usable: calling `show()` again opens a new instance.
 * Work running in the window (polling, timers) should be stopped in {@link Window.onClose}.
 *
 * `dispose()` equals `close()` plus releasing the handle: afterwards `show()` does nothing, and the id can be used in a new `create`.
 *
 * **Testing**: `host.window(id).props` reads the current properties, `host.window(id).userClose()` simulates the user closing it.
 * See {@link "@vestra-player/sdk/testing"!TestHost}.
 *
 * @capability ui.window
 * @since 1.0.0
 * @category UI
 */
export interface Window extends View, Disposable {
  /**
   * Open the window (on the first call, or after `close()` / the user closing it), or show a hidden window again.
   *
   * @remarks
   * On opening, the current `bind` values and the most recent `set` values are used as the initial properties.
   * `show`, `hide` and `close` on the same window run strictly in call order; several concurrent `show()` calls share one open.
   * Calling `show()` after the handle is disposed does nothing and resolves immediately.
   *
   * @returns Resolves when the window is on screen.
   *
   * @throws `capability` The manifest does not declare `ui.window`.
   * @throws `unavailable` The player is running in headless mode.
   * @throws `invalid` `file` goes outside the plugin package directory. [To verify: the code comes from `plugins::package_file`]
   * @throws `missing` `file` does not exist. [To verify]
   * @throws `host` The `.slint` file failed to compile or the component does not follow the rules; `message` is the compiler output.
   *   [To fix: `host` is not in the {@link PluginError} error table; add it to the table, or unify it as `ui-prepare`]
   *
   * @example Tell the user when opening fails
   * ```ts
   * try {
   *   await info.show();
   * } catch (error) {
   *   void plugin.osd.show(`Cannot open the window: ${(error as PluginError).code}`);
   * }
   * ```
   */
  show(): Promise<void>;
  /** Hide the window; state and properties are kept, and `show()` shows it again unchanged. `opened` stays `true`. */
  hide(): Promise<void>;
  /**
   * Close and destroy the window instance; the handle is kept, and a later `show()` opens a new instance with the latest properties.
   * Does not call {@link Window.onClose}. Does nothing when the window is not open.
   */
  close(): Promise<void>;
  /** Whether the window instance exists (shown or hidden both count). `false` before the first `show()`, after `close()`, and after the user closes it. */
  readonly opened: boolean;
  /**
   * The user closed the window (the close button in the title bar, or the standard close keys ⌘W / Ctrl+W / Ctrl+F4).
   *
   * @remarks
   * When the listener is called, `opened` is already `false`. It is not called when the plugin itself calls `close()` or `dispose()`.
   * Errors thrown in the listener go to {@link Scope.onError}.
   *
   * @returns A handle for removing the listener.
   *
   * @example Stop polling when the window closes
   * {@includeCode ../../examples/snippets/windows.ts#stop-polling}
   */
  onClose(listener: () => unknown): Disposable;
}
````

`examples/snippets/windows.ts` and `examples/snippets/ui/info.slint` are both real files that CI compiles and runs.

````ts
import type { Plugin } from "@vestra-player/sdk";

export default async function activate(plugin: Plugin) {
  // #region info-window
  const info = plugin.windows.create("ui/info.slint");
  info.bind(() => ({ source: plugin.player.state.source ?? "(no source)" }));
  info.on("copySource", (text: string) => plugin.clipboard.writeText(text));
  plugin.commands.add({
    id: "info",
    path: ["Tools", "Media Info…"],
    run: () => info.show(),
  });
  // #endregion info-window

  // #region stop-polling
  let timer: NodeJS.Timeout | undefined;
  const refresh = async () => info.set({ stats: JSON.stringify(await plugin.player.stats()) });
  info.on("startPolling", () => { timer ??= setInterval(refresh, 1000); });
  info.onClose(() => { clearInterval(timer); timer = undefined; });
  // #endregion stop-polling
}
````

Compared with the original, this rewrite adds:

- the concept definition: an independent top-level window, non-modal, size from Slint;
- how to choose between it and panels, settings pages and dialogs;
- the complete lifecycle, and the standard close keys;
- error codes (exposing two inconsistencies that need code fixes);
- how to test it;
- self-contained, compilable examples, and the matching `.slint` file;
- `@since`, `@category`, `@see`.

The total length of the reference entry is about 4 times the original. Following VS Code's experience, longer teaching material should go in the two pages "Concept: UI overview" and "Slint bridge", rather than piling more into the comments.

---

## 4. Toolchain

### 4.1 Changes to `typedoc.json`

```jsonc
{
  "lang": "zh",                                  // built-in Chinese UI
  "projectDocuments": ["docs/**/*.md"],          // tutorials, concepts, how-to guides; organized with frontmatter group/category/children
  "categorizeByGroup": false,
  "navigation": { "includeCategories": true, "includeGroups": false, "includeFolders": true },
  "defaultCategory": "Other",
  "categoryOrder": ["Core", "Commands and shortcuts", "UI", "Playback", "Processing", "System", "Plugin management", "Testing", "*"],
  "blockTags": ["@capability", "@player" /* plus TypeDoc's default tags */],
  // also declare @capability and @player in tsdoc.json so that editors and lint recognize them
  "searchInComments": true,
  "searchInDocuments": true,
  "plugin": ["typedoc-plugin-mdn-links", "typedoc-github-theme"],
  "sidebarLinks": { "Error codes": "documents/error-codes.html", "Capabilities": "documents/capabilities.html" },
  "sort": ["source-order"]                       // keep: source order is "most used first"
}
```

- Add `@category` to every exported type (about 100 places, mechanical work).
- `@capability` is rendered as its own section through `blockTags`. To make it a badge, write a `customJs` of about 30 lines, or a TypeDoc plugin that renders it below the signature.
- A custom `@player 0.2.0` replaces the "Requires player 0.2.0" phrases in the body text.

**Theme**: `typedoc-github-theme` first. It changes little, keeps up with 0.28, looks close to GitHub, and matches the look of the repository.

**Another route (to consider in the medium term)**: `typedoc-plugin-markdown` plus `typedoc-vitepress-theme`, generating Markdown for VitePress.

- Benefits:
  - better layout, table of contents and full-text search for hand-written pages;
  - native support for multilingual sites (`/` Chinese, `/en/` English);
  - a unified "API reference + guides" navigation, closest to the VS Code or MDN experience.
- Cost: one more build layer, and some of TypeDoc's validation has to run after the Markdown output.

Recommendation: finish the content rewrite on TypeDoc's default HTML first; **do content and toolchain in two separate steps**.

### 4.2 Examples must compile and run

Three layers, from easy to hard:

1. **`examples/` is a real TS project**:
   - `examples/snippets/*.ts`, cut into fragments with `// #region name`; the matching `.slint` files go in `examples/snippets/ui/`;
   - add a `tsconfig.examples.json` and run `tsc --noEmit` on it as part of `npm run check`;
   - TSDoc and Markdown pages always include code with `{@includeCode ../../examples/snippets/x.ts#region}`.
   - This is OpenCV's `@snippet` approach. A type error or a mistyped region name makes `typedoc` fail directly, since `treatWarningsAsErrors` is already on.
2. **Extract-and-check for inline code blocks** (covering the existing 79 inline `@example`s and the code blocks in `docs/*.md`):
   - write a `scripts/check-examples.mjs` that:
     - walks the comments with `typedoc --json` or the TypeScript API and finds `` ```ts `` code blocks;
     - also scans `docs/**/*.md`;
     - writes each code block to `.tmp/examples/<symbol>-<n>.ts`, prefixed with a shared preamble:
       ```ts
       import type * as SDK from "@vestra-player/sdk";
       declare const plugin: SDK.Plugin;
       ```
       adding `declare const` for a small set of agreed variables when needed, but such variables should be eliminated as far as possible;
     - finally runs `tsc --noEmit`.
   - Fragments that are intentionally incomplete are marked explicitly with `` ```ts nocheck ``, and there should be as few of them as possible.
   - This layer is rustdoc's "examples are tests" (the compile part).
3. **Run-level doctest**: for every file in `examples/snippets` whose default export is `activate`, activate it once in `node --test` with `startPlugin` plus `TestHost`, and assert a few key side effects (command registered, `host.window(id)` exists, polling stopped after `userClose()`). The SDK ships its own test host, which is exactly what makes Rust's "examples are run" possible here.

**Slint fragments**: the SDK CI currently has no Slint compiler.

- One option is to compile `examples/snippets/ui/*.slint` in the player repository's plugin tests, run once before an SDK release;
- another is to bring in Slint's Node package or `slint-lsp` for syntax checking only. This can wait for phase two.

### 4.3 Link checking

- Internal links: `validation.invalidLink` already covers `{@link}`. TypeDoc 0.28 also validates relative links in hand-written Markdown, reporting failures as warnings, which in turn fail the build.
- External links: after `_docs.yml` generates `docs-site`, add a lychee step (`lycheeverse/lychee-action`) that scans the HTML, ignoring `localhost`. External links occasionally flap, so this is best run on a schedule (weekly); PRs run only the internal checks, so false positives do not block merges.
- Drift check: a small script asserts that the Node version in `docs/getting-started.md` equals `engines.node` in `package.json`, and that the SDK version in the examples equals the current major version. This is how the kind of drift in §1.2 ⑤ was found.
- Coverage: `typedoc-plugin-coverage` produces a badge. Also write a `scripts/doc-lint.mjs` that reads `typedoc --json` and enforces these rules:
  - every method that returns a `Promise` must have `@throws`;
  - every module entry must have `@remarks`, `@example`, `@capability`, `@category`;
  - every method must have `@since`.

### 4.4 Documentation language

Recommendation: **Chinese as the primary language, including the comments in the API reference**; keep an English introduction in the README and on the npm page. Reasons:

1. The product, the plugin store and the users are mainly Chinese-speaking. The most complete existing long documents (`reference.md`, `tutorial.md`, `examples.md`, about 2950 lines) are already in Chinese and need almost no translation to move over; conversely, translating them into English is the largest single piece of work.
2. The TSDoc language shows up directly in editor hover tooltips. Chinese plugin authors seeing Chinese descriptions in VS Code is the most direct benefit.
3. TypeDoc has built-in `lang: "zh"`, so the site's UI text can be Chinese throughout.
4. Code identifiers, error codes, capability names and Slint keywords all stay in English. UI text in the examples is Chinese, consistent with the player.

When English is needed, in increasing order of effort:

- (a) maintain only an English README and an English getting-started (one page), without translating the reference;
- (b) take the VitePress route, with an `/en/` subsite for English tutorials and concept pages, while the API reference stays in Chinese TSDoc;
- (c) fully bilingual: writing TSDoc twice is not realistic. A workable approach is to keep the reference comments in one language and cover only tutorials, concepts and how-to guides in the other.

Mixing Chinese and English in the same comment is **not recommended**.

If the team judges overseas developers to be an important audience: TSDoc in English, tutorials, concepts and how-to guides in Chinese, each as its own area of the site. The cost is that the links between reference and concepts cross languages.

---

## 5. Effort estimate

| Item | Count | Notes |
| --- | --- | --- |
| A: module entries and core types, rewritten to the full template (concept paragraph, choosing, lifecycle, overall example) | about 30 | Plugin, Scope, Disposable, PluginError, View, Windows, Window, Panels, Panel, PanelSlot, Settings, Dialogs, Osd, Ui, Drops, MainWindow, Commands, Command, CommandOptions, Bindings, Player, Media, Conditional, Stream, Subtitles, VideoFilter, AudioFilter, VideoView, MediaCapture, Packages, Resolvers, Storage, Secrets, System, Tools, plus TestHost from testing |
| B: methods, adding `@throws`, `@capability`, `@since`, and parameter ranges, units and defaults | about 137 (types.ts) + about 40 (testing) | About 90 need new or rewritten compilable examples; the one-line methods of Player, Packages and Subtitles are the most numerous |
| C: data fields, adding units, ranges and enum values | about 60 (out of about 390 members in total) | For example `checks: string[]`, `status: string`, the fields of `stats`; the rest are good enough |
| Fixing factual drift | about 6 places | Node version, SDK version number, error table missing `versionMismatch` / `host` / `ui-prepare`, duplicate window id without code |
| `@category` tags | about 108 places | Mechanical work |
| New or migrated concept pages | 9 | Mainly from `reference.md` §1–3, §6, §13, and some chapters of `tutorial.md` |
| How-to guides | about 20 | Merged and rewritten from the 34 sections of `examples.md`; all code moves into `examples/snippets` |
| Tutorials | 4 | Merged from `tutorial.md` and `getting-started.md` |
| Toolchain | typedoc configuration, `tsdoc.json`, `check-examples.mjs`, `doc-lint.mjs`, example doctests, lychee, theme | about 1–2 days |

Rough schedule:

- First finish the toolchain and the "UI" category (Windows, Panels, Settings, View: 8 type-A items plus their methods) as a model, and give it to the user to review the writing style.
- Then proceed category by category: playback, commands, processing, system, plugin management, testing. Each category delivers "concept pages + how-to guides + reference" together.

---

## 6. Reference links

- MATLAB `interp1`: https://www.mathworks.com/help/matlab/ref/double.interp1.html
- man-pages(7): https://man7.org/linux/man-pages/man7/man-pages.7.html
- rustdoc writing guide: https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html
- Rust `std::fs::File`: https://doc.rust-lang.org/std/fs/struct.File.html
- MDN `fetch()`: https://developer.mozilla.org/en-US/docs/Web/API/Window/fetch
- MDN `Window`: https://developer.mozilla.org/en-US/docs/Web/API/Window
- Stripe create refund: https://docs.stripe.com/api/refunds/create
- VS Code extension capabilities overview: https://code.visualstudio.com/api/extension-capabilities/overview
- VS Code Webview guide: https://code.visualstudio.com/api/extension-guides/webview
- `vscode.d.ts`: https://github.com/microsoft/vscode/blob/main/src/vscode-dts/vscode.d.ts
- Qt `QWidget`: https://doc.qt.io/qt-6/qwidget.html
- OpenCV Canny tutorial source: https://github.com/opencv/opencv/blob/4.x/doc/tutorials/imgproc/imgtrans/canny_detector/canny_detector.markdown
- OpenCV Python Canny tutorial source: https://github.com/opencv/opencv/blob/4.x/doc/py_tutorials/py_imgproc/py_canny/py_canny.markdown
- OpenCV `imgproc.hpp`: https://github.com/opencv/opencv/blob/4.x/modules/imgproc/include/opencv2/imgproc.hpp
- Django documentation home: https://docs.djangoproject.com/en/stable/
- Diátaxis: https://diataxis.fr/
- Node.js `fs`: https://nodejs.org/api/fs.html
- TypeDoc tags: https://typedoc.org/documents/Tags.html
- TypeDoc output options: https://typedoc.org/documents/Options.Output.html
- TypeDoc input options: https://typedoc.org/documents/Options.Input.html
- npm package versions (checked 2026-10-22): typedoc-plugin-mdn-links 5.1.1, typedoc-github-theme 0.4.0, typedoc-material-theme 1.4.1, typedoc-plugin-markdown 4.14.1, typedoc-vitepress-theme 1.1.4, typedoc-plugin-coverage 4.0.3
