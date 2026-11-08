# Online Subtitles

Searches the web for subtitles for the current video and loads them right away.

## Features

- OpenSubtitles.com (the default) and Assrt (射手网(伪)). Local files are searched by file hash and title; searching by title alone works too.
- Results are ordered by how well they match the file, human before machine translation, your preferred languages and the number of downloads; you can show only exact matches with the file, or exclude keywords.
- Double-click loads a subtitle and saves a copy beside the video; you can also only add, only save, or save as. When an archive holds several subtitles, you choose one.
- Optional: search automatically when you open a local video without subtitles, and load the result automatically or show the results.

## Where to find it

Right-click menu, Open ▸ Search and Download Subtitles… (the last item before Recently Played); command palette (Ctrl+Shift+P): Online Subtitles: Online Subtitle Settings…

## Before you start

- You need your own API key from the chosen service (enter it in Online Subtitle Settings…). Without a key nothing is sent.
- Before the first search you are told that searching sends the file name keywords and the file hash to the chosen service.
- Keys and passwords are kept in the system's secret store.

## Limits

- No rar or 7z archives; no uploading of subtitles.
