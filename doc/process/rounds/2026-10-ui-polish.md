# 2026-10 round: plugin center and UI polish

Registered 2026-10-25. **This document is the only progress source for this round.**

**Start condition**: all other 0.5.0 work is done, including i18n round 2, translation and verification of the five languages, and the host fixes. Until then this round is only registered and no work is assigned.

## The user's requirements (key points of what the user said)

- The plugin store (the interface that can show Markdown) takes over all plugin management functions; disabling, uninstalling and other actions are all done there.
- Categories such as "Installed / Not installed" belong in a tab bar.
- Entries such as repository management and manual install become one icon that opens a menu, or each item is an icon of its own.
- Each row of the plugin list has install and uninstall buttons, like VS Code, and shows the plugin's logo.
- After that, find every unreasonable part of the interface and fix it. Known examples:
  - the AI workshop buttons are too large and the layout is unreasonable;
  - the drop-down menus for the API settings on the Vestra Player AI settings page have the wrong style.
- The user's criticism: these problems should have been noticed while writing, or spotted by ourselves, instead of waiting for the user to point them out.

## Why they were missed (retrospective)

- Author agents cannot see the rendered result when writing Slint; they can only confirm that the code compiles and the tests pass.
- The coordinator's real-machine screenshots covered only the changed features, without a visual review of each interface.
- Authors did not check their work against ui-spec and the existing interfaces, which led to oversized buttons and self-drawn drop-down boxes.

The countermeasures are all in the process below; the key point is that whoever writes an interface has **looked at** the rendered result themselves before delivering.

## Process

### Scope (user ruling, 2026-10-25)

**Do not redesign the interface** (不重新设计界面). The existing interface was researched and settled; the standard is the existing [ui-spec.md](../../product/ui-spec.md). This round does only two things:

- the plugin center changes the user named (step 1), limited to the points the user listed;
- find and fix the ugly parts (step 2): code that violates ui-spec, clearly mismatched sizes and layout (for example oversized buttons), and controls whose style differs from the other interfaces (for example self-drawn drop-down boxes). The fix is to align with ui-spec and the existing interfaces; no new specification, no changes to the theme, colours or layout structure.

### Step 0: preparation (maintainer)

- **Offscreen screenshot tool**: one command renders every window and settings page of a given plugin to PNG, with a chosen language (en, zh-Hans, de) and sample data. Built by extending the existing slint-check prototype. Author agents look at the images with the Read tool.
- From now on, task instructions include one line: before delivering, look at en and de with the screenshot tool and check against ui-spec.

### Step 1: plugin center (merging the store and plugins plugins)

1. **The coordinator writes the product specification**, below in this document, with wireframes; it keeps the store's existing window, colours and controls and changes only what the user named:
   - one window, with tabs at the top: Installed, Available, Updates;
   - each list row: logo, name, author, version, an inline primary button in the style of the VS Code extension list (Install / Uninstall / Update) using the existing Button component; Disable goes in the row's secondary actions;
   - details (Markdown) on the right;
   - a gear or "…" icon at the top right, whose menu has repository management, manual install (sideloading) and open data folder.
2. **The store author and the plugins author** negotiate according to §4: which functions move into store, whether the plugins plugin is retired or keeps only an entry, and the menu path (whether to keep "Settings ▸ Plugin Manager…"). Host or SDK capabilities they need, such as icon buttons, a tab bar component or an API to read plugin logos, are requested from the maintainer according to §4.1.
3. Implementation → the author checks with their own screenshots → the coordinator accepts with screenshots (real Mac) → the user looks on Windows.

### Step 2: full interface review

1. **The coordinator takes screenshots of every interface**: the main window, each menu, every page of the Options window, every plugin window and sidebar panel, in en, zh-Hans and de (the longest). On a real Mac; for Windows, ask the user for a few key screenshots.
2. **Interface reviewer** (a temporary role, an independent agent that writes no code):
   - reviews each screenshot against ui-spec and the other interfaces, records only ugly problems, and makes no "could be designed better" suggestions;
   - each problem states the screenshot, the location, which ui-spec rule it violates (or which interface it is inconsistent with), and the severity (clear error / mismatched / minor flaw);
   - the problem list goes below in this document.
3. The coordinator rules: confirm, reject, merge duplicates, then assign the confirmed problems to authors by plugin, and host component problems to the maintainer.
4. The author fixes them and attaches before and after screenshots → the reviewer reviews again → the coordinator accepts on the real machine.
5. Problems not solved within two rounds are listed for the user to decide.

### Out of scope

- No interface redesign and no new interface specification. There is only one reference: the plugin center's list rows may follow the VS Code extensions view (named by the user). No other interface uses any software as a reference (the reference research has been done; its conclusions are the current ui-spec and interface), and task instructions do not mention any.
- No changes to interaction logic or feature scope, except the parts of the step 1 plugin center the user named.
- No changes to the theme and colours. If the review finds a problem in ui-spec itself, only register it and ask the user.

## Progress

- [ ] Step 0: screenshot tool
- [ ] Step 1: plugin center specification → negotiation → implementation → acceptance
- [ ] Step 2: full screenshots → review → fixes → second review → acceptance
