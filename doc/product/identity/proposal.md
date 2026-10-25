# Visual identity proposal (A1)

**On 2026-10-24 the user chose direction A, "V-Line × Aurora", and asked for A′, "Aurora Light", to be selectable as a theme; this has been implemented, see "Implementation" at the end.** Below is the original text of the options offered at the time.

Start with `compare.png`: one row per option, in order current, A, B, C and A′ light, all rendered at 1× actual size. Per-element comparisons (2×): `compare-bar.png` (bottom bar normal and hover), `compare-icons.png` (all icons, 1× and 3×), `compare-sidebar.png`, `compare-menu.png`, `compare-osd.png`, `compare-options.png`, `palettes.png`.
All screenshots were drawn with the software renderer by the real Slint 1.18 interpreter loading the real `@vestra` library and plugin .slint files.

## What stays the same

The size and position of buttons, menu items, the bottom bar, the sidebar and the options window, the menu layout, and every icon's drawing box (viewbox) all stay as they are. The new icons can be dropped in directly (see `icons-*.json`), and the palette tokens map one to one to `theme.rs` / `palette.slint` (see `palette-*.json`).

## Three directions

**A · V-Line rounded lines × Aurora**
- Icons: uniform 1.5px lines with round caps and rounded corners; 1.25px for small icons up to 12px.
- The V shape of the logo runs through the whole icon set: rewind and fast-forward are « », move up and move down are ∧ ∨, the menu arrow, drop-down arrow and scroll arrows are all small Vs, and the playlist toggle also ends in a small V.
- The palette takes the logo's full spectrum: magenta is the accent (the centre of the V), cyan means "now playing" (the right end of the V), and the seek bar is a gradient from amber to cyan.

**B · Soft Solid × Violet Night**
- Icons: rounded solid shapes, matching the thick, rounded V of the logo itself; fast-forward, rewind, move up and move down are all solid thick V shapes.
- Play/pause becomes a 28px purple circular primary button, but the button cell is still 41×40.
- The palette takes the cooler half of the logo: purple accent, magenta for now playing; softer overall.

**C · Geometric thin lines × Graphite**
- Icons: 1px thin lines, square caps, sharp corners, using only straight lines, circles and 45°/90° arrows, for a "precision instrument" feel.
- The background is a neutral graphite grey, and colour appears only where it means something: cyan accent, magenta for now playing, and the seek bar is a gradient from magenta to cyan that ends exactly on the accent colour.
- Drawback: at 100% scaling on Windows, 1px lines must be pixel-aligned to look sharp, and each one has to be calibrated during implementation.

**A′ light (optional)**: a light version of A, with the video area kept dark. Note also that std-widgets use the fluent-dark style, so scroll bars and other controls stay dark under a light theme and need separate handling.

## Changes other than icons (same sizes, appearance only)

| Element | Current | A | B | C |
|---|---|---|---|---|
| Separators between bottom-bar buttons | Full-height black lines | Removed | Removed | 14px short lines, lighter colour |
| Button hover | Whole cell filled as a square | Centred 32×32 rounded background | Centred 34×30 rounded background; the play button's circle lights up with a gradient | No fill: the icon turns the accent colour and a 16px gradient dash appears below it |
| Black lines above and below the seek bar row | Two, top and bottom | Only a thin light line at the top | No lines | Only a thin light line at the top |
| Seek bar and volume thumbs | 8px white dot, near-black track | White core + 2px accent ring | Solid accent dot | 2×10 vertical playhead |
| OSD | Bold white text + black outline (the reference player's signature style) | Dark translucent rounded plate, with a gradient line along the bottom edge | Capsule, with an accent dot in front | Dark square plate, with a 2px accent bar on the left |
| Playlist tabs (105×31) | Colour blocks | 28px gradient dash under the text | Rounded capsule | Full-width 2px gradient underline |
| Now-playing row | Coloured text | Coloured text + 2px bar on the left | Coloured text; the selected row is a rounded block | Coloured text |
| ADD / DEL / SORT | English capitals | ＋ − ⇅ icons (capsule width unchanged, with hover tooltips) | Same as A | Same as A |
| Menu check / radio / submenu arrow | The characters ✓ › ▲▼▾ from the font | Replaced with icons from this set; check is a rounded square, radio is a ring | Check is a circle, radio is a solid dot | Lines only; radio is a square dot |
| Corner radius radius-control / radius-panel | 3 / 6 | 4 / 8 | 6 / 10 | 2 / 4 |
| Separator colour `line` | Near black | A thin line slightly lighter than the background (all options) | Same | Same |

Changing ADD / DEL / SORT to icons is a separate, optional choice; the text can also be kept (for example changed to the Chinese "添加/删除/排序" (Add/Delete/Sort), with the same width).

## Recommendation

**Choose A (V-Line × Aurora).**
- The most direct relationship with the logo: both the V shape and the full spectrum come from it.
- The clearest difference from the reference player: the solid triangles, the eject icon, the separated cells and the outlined OSD are all gone.
- Line icons speak the same language as the existing detach, close and search icons, so the amount of change is smallest.
- The most balanced contrast of the three: body text 16.6:1, accent 5.1:1, now playing 7.7:1.

A compromise to consider: borrow only B's "28px circular primary play button" into A, which makes the play button stand out more. The light theme can be done when needed and does not block this round.


## Implementation (2026-10-24, player 0.4.0, SDK 1.5.0)

Final result: [final-dark.png](final-dark.png) (`vestra`) and [final-light.png](final-light.png) (`vestra-light`), drawn at 2× by the Slint 1.18.1 software renderer loading the real library and plugin .slint files. The specification is in the [UI spec](../ui-spec.md) §2.1 (all tokens of both palettes), §2.5 (icon rules and list), §4–§7 and §9 (on-screen messages).

- **Palette**: `vestra` is replaced with Aurora (`palette-aurora.json`), and a new palette id `vestra-light` (`palette-aurora-light.json`) is added: the two tables in the host's `theme.rs` and the two gradient constants in `ui/palette.slint`; "Settings ▸ Appearance ▸ Vestra Dark / Vestra Light" and the appearance page of the options window (with a preview); the choice is stored in `ui-state.json`; the SDK's `ThemeId` gains `"vestra-light"`.
- **std-widgets in light mode**: the fluent style in Slint 1.18 reads `Palette.color-scheme` at run time (`fluent-dark` only decides its initial value), so the host still compiles with `fluent-dark`, two-way binds `color-scheme` in `VestraPalette` to the std-widgets' `Palette.color-scheme`, and sets it to `light` when switching to the light palette. Input boxes, drop-downs, check boxes and scroll bars become light along with it, without recompiling or replacing controls. The remaining difference: fluent controls use their own neutral grey and blue accent, the same under both palettes.
- **Icons**: a new library file `@vestra/icons.slint` (`Icon`, `Icons`, derived from `icons-v-line.json`), used throughout the bottom bar, playlist, chapters/bookmarks, shortcuts, store, Markdown task boxes, menu check/radio/submenu arrows/scroll arrows, drop-down arrows, sort arrows, tag ×, and the appearance preview; the font characters ✓ › ▲▼ ▾ × ▶ are no longer used as icons. ADD / DEL / SORT become 41×18 icon buttons (＋ − ⇅, with the hover tooltips "Add", "Delete", "Sort").
- **Other appearance**: the bottom bar loses the full-height separators between buttons and the line under the seek bar row, and hover is a 32×32 rounded background; the seek bar and volume thumbs are a white core + 2px accent ring; playlist tabs and tool window tabs are a 28px gradient dash under the text; the now-playing row has a 2px cyan bar on the left; the current menu item has a 2px accent bar on the left, and separators are indented; the selected item in the options tree is a bar + body text colour; on-screen messages and the info overlay are translucent dark rounded plates, with a gradient line along the bottom edge of on-screen messages; corner radius 4/8px.
- **Geometry**: the size and position of buttons, hit areas and menu items, and the menu layout, are unchanged. How it was checked: the Slint 1.18.1 interpreter loaded the .slint files before the change (main 4d2759e) and after it, built 27 scenes with the same data and window sizes (bottom bar normal/hover/narrow window/idle, four playlist scenes, three menus, the options window, two shortcut pages, the mouse page, two store pages, audio, picture, chapters/bookmarks, playback properties, shell on-screen message), read the absolute position and size of every TouchArea / FocusScope / TextInput / element with an accessibility role through the test backend's `ElementHandle`, and compared them one by one; in addition, every button in the bottom bar and the playlist was clicked, and the triggered callbacks compared. Result: identical except in two places. (1) The appearance page has one more row (the light palette), so "Restore Defaults" below it moves down one row; this is new content. (2) Before the change, the six icon buttons of the playlist — move to top/move up/move down/move to bottom, detach and close — had a 0×0 hit area (the tooltip component had no size set), so clicking them did nothing; they now fill the 25px slot and can be clicked. The only places where text boxes changed: the drop-down arrow slot changed from the width of the ▾ glyph (5px on this machine) to the icon width of 8px, so the text in drop-downs is 3px narrower; the replaced glyph text no longer exists.
