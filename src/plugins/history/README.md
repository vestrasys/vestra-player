# Playback History

Remembers what was played recently, and continues next time where you stopped.

## Features

- **Recently Played**: lists the 20 files and links opened most recently; click one to open it again. Invalid entries can be removed, the list cleared, or recording turned off.
- **Resume Playback**:
  - Don't Resume;
  - Resume Automatically: opened again, it continues from the last position;
  - Ask Every Time: on opening, asks whether to Continue or Start Over.
- By default only videos are remembered; audio files can be included. Sources shorter than a minute, or almost played to the end, are not remembered.

## Entry points

- Context menu Open ▸ Recently Played (the last line of Open ▸).
- Command palette (Ctrl+Shift+P): Resume Playback: Don't Resume / Resume Automatically / Ask Every Time, Also Remember Audio Files, Clear All Saved Positions (no longer in the right-click menu).

## Limits

- Once a file is modified (its size changes), its saved position is dropped.
