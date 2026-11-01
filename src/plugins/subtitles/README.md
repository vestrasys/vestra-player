# Subtitles

Picks subtitle tracks, loads external subtitles, adjusts style and sync.

## Features

- **Tracks**: show/hide, pick a track or cycle through them.
- **External subtitles**: load SRT, ASS/SSA, WebVTT, SAMI and MicroDVD, with the encoding detected; reload them, automatically when the file changes if wanted.
- **Same-name subtitles**: when a local video opens, subtitle files next to it with its name (`movie.srt`, `movie.en.srt`, `movie.zh-Hans.ass`) load automatically (at most 2, the exact name first), the first shown unless you chose a track; on by default (Subtitle Settings…). Refresh Subtitle Tracks also loads files added later.
- **Secondary subtitle**: a second text subtitle at the top or above the main one.
- **Style**: size, position, font, colour, outline, bold, shadow, background box, and whether ASS/SSA keep their own styles; the style is kept across restarts.
- **Sync**: earlier, later, reset; a sync step, or a value typed in milliseconds.
- **Jump by dialogue**: previous, next, start of the current subtitle.

## Entries

- Right-click menu: Open ▸ Open Subtitle… (Alt+O), Open ▸ Add Secondary Subtitle….
- Right-click menu Subtitles: Show Subtitles (Alt+H), Subtitle Track (Cycle Alt+L), Secondary Subtitle, Subtitle Style (Bold Alt+B, Size Alt+PgUp/PgDn, Move Up/Down: no default key, Alt+↑/↓ pan the video), Sync (`.` earlier, `,` later), Reload Subtitles (Ctrl+Alt+Y), Subtitle Tuning…, Subtitle Settings….
- Right-click menu Reset: Subtitle Position (Alt+Home), Subtitle Sync (`/`).
- Right-click menu Playback ▸ Seek: Previous Dialogue (Home), Next Dialogue (End), Current Subtitle Start (Ctrl+Home).

## Limits

- No image subtitle files (.sup, VobSub).
- The menu lists at most 8 tracks; the rest are in Subtitle Tuning….
- Jumping by dialogue works for text subtitles of local files only.

## Translations

Texts are English in the source; `locales/zh-Hans.po` has the Simplified Chinese (`vestra-plugin i18n extract`, `i18n check`). The windows' status line and the on-screen messages share one table, `messages.ts` (by id); both get the same text value (`statusText`), which the player translates.
