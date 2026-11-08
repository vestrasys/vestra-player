# Subtitles

Picks subtitle tracks, loads external subtitles, adjusts style and sync.

## Features

- **Tracks**: show/hide, pick a track or cycle through them.
- **External subtitles**: load SRT, ASS/SSA, WebVTT, SAMI and MicroDVD, with the encoding detected; reload them, automatically when the file changes if wanted.
- **Same-name subtitles**: when a local video opens, subtitle files next to it with its name (`movie.srt`, `movie.en.srt`, `movie.zh-Hans.ass`) load automatically (at most 2, the exact name first), the first shown unless you chose a track; on by default (Subtitle Settings…). Refresh Subtitle Tracks also loads files added later.
- **Secondary subtitle**: a second text subtitle at the top or above the main one.
- **Style**: size, position, font, colour, outline, bold, shadow, background box, and whether ASS/SSA keep their own styles; the style is kept across restarts.
- **Sync**: earlier, later, reset; a sync step, or a value typed in seconds (`-1.25`, `0,5 s`, `250 ms`).
- **Jump by dialogue**: previous, next, start of the current subtitle.

## Entries

- **Subtitles panel** (adjust column beside the video): right-click menu **Subtitles…** (Alt+3; again closes the column), or the bottom bar's Adjust button. One column: Show subtitles; Tracks (Primary, Load…, Reload, Secondary and its position); Sync (−/+ by the sync step, the value — click it to type one —, reset; Previous / Next line); Style (size, position, bold, shadow, box, Keep original ASS/SSA styles, "Font, Colour and Outline…" for Subtitle Settings); Reset. Each control's key is shown beside it and in its tooltip, as the user has bound it.
- Right-click menu: Open ▸ Open Subtitle… (Alt+O), Open ▸ Add Secondary Subtitle…, Subtitle Track ▸ (Off and the tracks), Subtitles… (Alt+3).
- Command palette (Ctrl+Shift+P) and keys: everything else, as "Category: Title" — Show Subtitles (Alt+H), Cycle (Alt+L), Previous Dialogue (Home), Next Dialogue (End), Current Subtitle Start (Ctrl+Home), the secondary subtitle and its position, Size +/− (Alt+PgUp/PgDn), Move Up/Down (no default key; Alt+↑/↓ pan the video), Bold (Alt+B), Shadow, Opaque Box, Original ASS/SSA Styles, Earlier / Later (`.` `,`), Adjust Sync… (opens the panel's sync value for typing), Reset: Subtitle Position (Alt+Home), Reset: Subtitle Sync (`/`), Reload Subtitles (Ctrl+Alt+Y), Refresh Subtitle Tracks, Remove External Subtitle, Subtitle Settings….

## Limits

- No image subtitle files (.sup, VobSub).
- Subtitle Track ▸ lists at most 8 tracks; the rest are in the panel's Primary list.
- Jumping by dialogue works for text subtitles of local files only.

## Translations

Texts are English in the source; `locales/<lang>.po` translate them (de, es, fr, ja, ko, zh-Hans) (`vestra-plugin i18n extract`, `i18n check`). The panel's and the settings window's status line and the on-screen messages share one table, `messages.ts` (by id); both get the same text value (`statusText`), which the player translates.
