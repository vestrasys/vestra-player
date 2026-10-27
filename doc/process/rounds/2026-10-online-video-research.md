# Research: online video in a sidebar, with no yt-dlp install (VestraTube, VestraBili)

Researcher report, 2026-10-26. Read-only; no code was changed and nothing was filed.

Sources:
- Our code: player `C:\PROJECT\vestra-player` at `19fd7b6`. Paths below are relative to `src/`.
- SDK 1.5.17 docs: `src/node_modules/@vestra-player/sdk/llms-full.txt`, cited as `SDK:<line>`.
- Plugins repository: shallow clone at `C:\vestra\plugins-repo`, cited as `plugins-repo/...`.
- Live probes: anonymous GET requests made today. The Bilibili probes ran from an IP that `/nav` reported as `ip_region: DE`. YouTube and Bilibili thumbnail probes ran from this machine.

Items I could not verify are marked **[unverified]**.

## Summary

- **Both sites can be listed and played from plain Node with no yt-dlp.** The constraints differ:
  - **Bilibili is easy and fairly stable.** It needs about 20 lines of WBI signing and a Referer/User-Agent pair, and the host already supports both. Anonymous playback gives 720p MP4, or DASH at 480p, and sometimes 1080p with `try_look=1`.
  - **YouTube is feasible but fragile.** The recommended library is **youtubei.js 18.3.0 (MIT)**, with the `VISIONOS` client for URLs that need no PO token and our own player-JS evaluator in a worker. YouTube changes every few weeks. yt-dlp's maintainers expect logged-out extraction without a JS runtime to stop working eventually, and IP-based "not a bot" checks cannot be solved without a login.
- **The host already plays what a resolver returns:** URL plus headers, plus separate audio as `edl://` with `http-header-fields` (`host/src/source_resolvers.rs:307-353`). No mpv/yt-dlp integration is needed. Our libmpv contains `ytdl_hook`, which runs an external yt-dlp, and the host turns it off (`host/src/backend.rs:1141-1150`).
- **Biggest API gap:** an image cannot be placed in a list row (`SDK:859`), so a list of thumbnails is not directly possible. Two more gaps:
  - the plugins repository has no setup for a hosted plugin's own npm dependencies;
  - a stream URL that expires in the middle of playback has no re-resolve path **[unverified]**.
- **Effort:** about 6–9 developer-days for both plugins, plus about 2–3 maintainer-days for the image-in-rows request and about 1 infrastructure-day.

---

## 1. How mpv does it, and whether our libmpv can do it without yt-dlp

- **ytdl_hook calls an external program.** mpv's `player/lua/ytdl_hook.lua` searches `paths_to_search = {"yt-dlp", "yt-dlp_x86", "youtube-dl"}` ([L18](https://github.com/mpv-player/mpv/blob/dc674bb200e3de43cf7b6feb26403812d87ac30f/player/lua/ytdl_hook.lua#L18)). It runs the program with `mp.command_native({_name = "subprocess", …})` ([L121-128](https://github.com/mpv-player/mpv/blob/dc674bb200e3de43cf7b6feb26403812d87ac30f/player/lua/ytdl_hook.lua#L121-L128)), passing `-J --flat-playlist …` ([L915-916](https://github.com/mpv-player/mpv/blob/dc674bb200e3de43cf7b6feb26403812d87ac30f/player/lua/ytdl_hook.lua#L915-L916)).
  - mpv's manual says `--ytdl` "requires a recent version of youtube-dl to be installed on the system" ([options.rst](https://github.com/mpv-player/mpv/blob/master/DOCS/man/options.rst)).
- **How it plays separate audio and video.** For split DASH formats it builds `edl://` with a `!new_stream;!no_clip;!no_chapters` header per track ([L548-636](https://github.com/mpv-player/mpv/blob/dc674bb200e3de43cf7b6feb26403812d87ac30f/player/lua/ytdl_hook.lua#L548)). It copies yt-dlp's `http_headers` into `file-local-options/http-header-fields` and `user-agent` ([L141-159](https://github.com/mpv-player/mpv/blob/dc674bb200e3de43cf7b6feb26403812d87ac30f/player/lua/ytdl_hook.lua#L141-L159)).
  - mpv documents `!new_stream` and `!no_clip` as internal and liable to change ([edl-mpv.rst](https://github.com/mpv-player/mpv/blob/master/DOCS/edl-mpv.rst)).
- **Our shipped libmpv has the same hook.**
  - `C:\vestra\deps\libmpv-2.dll` is shinchiro build `git-e471496c56` (`C:\vestra\deps\components.json`).
  - It embeds the hook. A byte search of the DLL finds `paths_to_search = {"yt-dlp", "yt-dlp_x86", "youtube-dl"}` and the `subprocess` command.
  - So it can only use an *installed* yt-dlp; there is no built-in extractor.
- **The host turns the hook off on purpose.** `BUILTIN_SCRIPTS_OFF` includes `"ytdl"`, with the comment "network sources are resolved by plugins" (`host/src/backend.rs:1141-1170`). There is a test for it (`backend.rs:1183-1199`).
- **What the host does with a resolved source** (`host/src/source_resolvers.rs:307-353`):
  - one URL, or `edl://!no_clip;!no_chapters;%len%video;!new_stream;!no_clip;%len%audio;` for separate audio;
  - headers become a per-file `http-header-fields` option;
  - audio and video must use *identical* headers (`:318-323`, and SDK `ResolvedMedia`, `SDK:6515-6533`). Both sites use one header set for both streams, so this is not a problem.
- **Conclusion:** mpv cannot play YouTube or Bilibili pages without yt-dlp. The resolver path that replaces it already exists. The plugins only have to produce stream URLs and headers in TypeScript.

## 2. YouTube from pure JS/TS

### 2.1 Libraries

Registry data comes from `https://registry.npmjs.org/<pkg>` (fetched 2026-10-26). Repository status comes from the GitHub API.

| Package | License | Latest | Maintenance | Runtime deps / unpacked | Fit |
|---|---|---|---|---|---|
| **youtubei.js** (LuanRT/YouTube.js) | MIT | 18.3.0, 2026-10-23 | Active (v16 2025-10, v17 2026-03, v18 2026-08) | `meriyah`, `fflate`, `@bufbuild/protobuf`; 16 MB unpacked across several builds (bundled size not measured) | **Recommended** |
| @distube/ytdl-core | MIT | 5.0.2, 2026-10-26 (rewrite released 2026-10-22) | Active, brand-new rewrite | `jsdom` 30 (5.5 MB, ~20 deps), `meriyah`, `bgutils-js`, `googlevideo` | Watch; too new, heavy, no feed API |
| ytdl-core (fent) | MIT | 4.11.5, 2023-07-14 | Paused; points to the distube fork ([repo](https://github.com/fent/node-ytdl-core)) | — | No |
| @ybd-project/ytdl-core | MIT | 6.0.8, 2024-11-13 | Repository **archived** (510ren/ytdl-core) | jsdom, undici | No |
| play-dl | **GPL-3.0** | 1.9.7, 2023-09-24 | Repository returns 404 | — | No: dead, and GPL cannot go into Apache-2.0 code ([Apache](https://www.apache.org/licenses/GPL-compatibility.html)) |
| googlevideo (LuanRT) | MIT | 5.0.1, 2026-10-23 | Active | SABR/UMP protocol | Only if YouTube forces SABR (§2.4) |
| bgutils-js (LuanRT) | MIT | 4.0.3, 2026-08-04 | Active | PO-token helper; "does not bypass BotGuard" ([README](https://github.com/LuanRT/BgUtils)) | Not needed on VISIONOS today |
| youtube-dl-exec | MIT | 3.1.17 | Active | Wraps the yt-dlp binary and needs Python | No: still yt-dlp |

- **Native modules:** none of these packages has one; all are JS. Node 26.12.1 ships with the player (`packaging/node.json:2`).
- **License compatibility:** youtubei.js is MIT and `meriyah` is ISC. Both are compatible with Apache-2.0 plugins and with the GPL-3.0 player.
- **yt-dlp in WASM:** I found no maintained yt-dlp-in-WASM/Pyodide package **[unverified; treat as not viable]**.

### 2.2 Search, feeds, thumbnails, durations (youtubei.js)

- **What it offers.** `Innertube.search()`, `getSearchSuggestions()`, `getHomeFeed()`, `getChannel()` and `getPlaylist()` exist ([Innertube.ts](https://github.com/LuanRT/YouTube.js/blob/main/src/Innertube.ts)). `Video` nodes carry `title`, `thumbnails`, `author`, `view_count`, `length_text` and `duration` ([Video.ts](https://github.com/LuanRT/YouTube.js/blob/main/src/parser/classes/Video.ts)).
- **Trending no longer exists.** YouTube retired the Trending page in July 2025 ([9to5Google](https://9to5google.com/2025/07/10/youtube-trending-page-removal/), [Tubefilter](https://www.tubefilter.com/2025/07/10/youtube-replacing-trending-tab-ranking-charts-popular-content/)). youtubei.js removed `getTrending` in v17 ([PR #1114](https://github.com/LuanRT/YouTube.js/pull/1114)). Charts are an open request ([#1261](https://github.com/LuanRT/YouTube.js/issues/1261)).
- **The logged-out home feed is [unverified].** YouTube is known to show a blank home feed when watch history is off ([Gigazine](https://gigazine.net/gsc_news/en/20230809-youtube-show-blank-page-none-watch-history)), so a guest session may get nothing. **Design for search first.** Treat "home" as a bonus that may be empty, or replace it with fixed search queries, for example per category.
- **Thumbnails need no API at all.** `https://i.ytimg.com/vi/<id>/mqdefault.jpg` returned 200 `image/jpeg`, 10 KB, 320×180 (live probe; `default`/`hqdefault`/`sddefault` also returned 200). That is JPEG, which is what `ui.image` accepts (`SDK:3609-3615`).

### 2.3 Stream URLs: deciphering, PO tokens, running player JS

- **youtubei.js does not run YouTube's player JS itself (since v16).**
  - The Node platform's default `eval` throws "To decipher URLs, you must provide your own JavaScript evaluator" ([jsruntime/default.ts](https://github.com/LuanRT/YouTube.js/blob/main/src/platform/jsruntime/default.ts), [#1052](https://github.com/LuanRT/YouTube.js/pull/1052)).
  - The official guide shows `Platform.shim.eval = async (data) => new Function(data.output)()` ([ytjs.dev](https://ytjs.dev/guide/getting-started.html)).
  - `Player.decipher` uses that evaluator for the signature and n-parameter and appends `pot=` when a PO token is given ([Player.ts](https://github.com/LuanRT/YouTube.js/blob/main/src/core/Player.ts)).
- **How we should run it.** A JS interpreter inside Node is fine, but the code comes from YouTube, and `node:vm` "is not a security mechanism" ([Node docs](https://nodejs.org/api/vm.html)). Run the evaluator in a `worker_threads` worker with a timeout and no access to the plugin's state. That is pure Node, with no OS command (workflow §1.5).
- **Which client to ask for URLs.** yt-dlp's current logged-out defaults are `tv_samsung`, `visionos`, `web`, with `visionos` as the only "JS-less" default. In its client table, `visionos` has `REQUIRE_JS_PLAYER: False` and no required PO-token policy ([_video.py L143-145](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/youtube/_video.py), [_base.py](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/youtube/_base.py)). youtubei.js has had a `VISIONOS` client since v18 ([Constants.ts](https://github.com/LuanRT/YouTube.js/blob/main/src/utils/Constants.ts)).
  - **[unverified]** that youtubei.js + VISIONOS returns direct playable URLs; it was not tested.
  - Known exception: made-for-kids videos return no formats on visionos ([yt-dlp #17871](https://github.com/yt-dlp/yt-dlp/issues/17871)). Fall back to `TV` or `WEB_EMBEDDED` there; the evaluator is needed for those.
- **Other clients are closed off:**
  - `web` serves only SABR.
  - `tv` serves DRM'd formats without cookies.
  - `mweb` and `web_creator` need a PO token.
  - Sources: [PO Token Guide](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide), [distube clients guide](https://distube.js.org/ytdl-core/guides/clients/).
- **"Sign in to confirm you're not a bot"** is tied to the IP: flagged VPNs and data-center addresses get it, and it cannot be solved anonymously ([yt-dlp #17904](https://github.com/yt-dlp/yt-dlp/issues/17904)). The plugin must show this as a clear state (§5.4), not retry.
- **How hard this is in general:** since 2025.11.12, yt-dlp itself expects an external JS runtime (Deno or Node). It says YouTube "will not be possible at all without a JS runtime" eventually ([#15012](https://github.com/yt-dlp/yt-dlp/issues/15012), [EJS wiki](https://github.com/yt-dlp/yt-dlp/wiki/EJS)). We would be tracking the same moving target with fewer maintainers.

### 2.4 Formats and mpv

- **Muxed:** only itag 18 (360p) is left, and it is reported unreliable ([yt-dlp #16150](https://github.com/yt-dlp/yt-dlp/issues/16150)) **[partly unverified]**.
- **Above 360p:** separate DASH video and audio. Return them as `primary` and `audio` in `ResolvedMedia`, and the host builds the EDL (§1). No plugin-side EDL is needed.
- **Codec choice:** prefer H.264/VP9 up to 1080p plus the AAC/Opus audio track. Respect the identical-headers rule: set one User-Agent matching the chosen client on both.
- **If YouTube makes every client SABR-only:** playback would need googlevideo plus a local proxy that turns SABR/UMP back into a stream mpv can read, plus BotGuard attestation, which is reported to fail outside a real browser ([YouTube.js #1123](https://github.com/LuanRT/YouTube.js/issues/1123)). Estimate that as weeks of work. It is the main long-term risk.

### 2.5 Recommendation (YouTube)

- **Library:** **youtubei.js 18.x**, bundled with esbuild (§4.3), pinned to an exact version.
- **Search and metadata:** `search()`; thumbnails built from `i.ytimg.com/vi/<id>/mqdefault.jpg`.
- **Streams:**
  1. Try `getInfo(id, { client: "VISIONOS" })` first.
  2. Fall back to `TV` / `WEB_EMBEDDED` with a worker-thread evaluator.
  3. Use itag 18 as the last resort.
- **Risks, in order:**
  1. Breakage on YouTube changes; plan for frequent releases (the plugin-repo rule of one version per change applies, `plugins-repo/CONTRIBUTING.md:56-60`).
  2. IP bot checks.
  3. SABR-only lock-down.
  4. Running YouTube's JS (isolate it).
  5. YouTube's terms of service forbid this kind of access. That is a policy decision for the project; I state it as fact, not as a ruling.
- **Fallback to keep:** an optional "use yt-dlp if one is installed" path (the current code). It would no longer be required, only a fallback.

## 3. Bilibili without login

### 3.1 Endpoints

- **Main reference, now archived.** The usual reference, SocialSisterYi/bilibili-API-collect (CC BY-NC 4.0), was archived on 2026-01-30 after a lawyer's letter from Bilibili ([repo README](https://github.com/SocialSisterYi/bilibili-API-collect)). Docs are cited below from a mirror ([rinnein fork](https://github.com/rinnein/bilibili-API-collect)). Do not copy its text into Apache-2.0 code; implement from behaviour.

| Purpose | Endpoint | Anonymous result (live probe) |
|---|---|---|
| Popular | `GET /x/web-interface/popular?pn&ps` | `code:0` with a browser UA; `-352` without a UA. Items have `bvid`, `cid`, `duration` (s), `owner.name`, `pic` (http) |
| Ranking | `GET /x/web-interface/ranking/v2?rid=0&type=all` | `code:0` with a browser UA |
| Search | `GET /x/web-interface/wbi/search/type?search_type=video&keyword&page&order` (WBI signed) | `code:0`. The old unsigned `/search/type` gives HTTP 412 without a `buvid3` cookie and 200 with a random `buvid3=<uuid>infoc`, which is what yt-dlp does ([bilibili.py L1887-1924](https://raw.githubusercontent.com/yt-dlp/yt-dlp/master/yt_dlp/extractor/bilibili.py)) |
| Video info | `GET /x/web-interface/wbi/view?bvid` (signed) | `code:0`. The unsigned `/view` was inconsistent (412 with a browser UA, 200 with curl's UA), which points to heuristic risk control |
| Parts list | `GET /x/player/pagelist?bvid` | `code:0`, `cid` and `duration` per part |
| Streams | `GET /x/player/wbi/playurl?bvid&cid&qn&fnval&fnver=0&fourk=1&try_look=1` | `code:0` signed and unsigned |
| Seek sprites | `GET /x/player/videoshot?bvid&index=1` | `code:0`; a 10×10 sheet of 160×90 tiles plus a timestamp index (see the seek-thumbnails report) |

- **Search result fields:** `title` contains `<em class="keyword">` tags to strip, `pic` is protocol-relative, `duration` is a `"13:42"` string, and the uploader is `author`.
- **WBI signing**, about 20 lines with Node's built-in `crypto` MD5 ([wbi.md mirror](https://raw.githubusercontent.com/rinnein/bilibili-API-collect/master/docs/misc/sign/wbi.md)):
  1. Get `img_key` and `sub_key` from `/x/web-interface/nav`. It answers `-101` when logged out but still includes `wbi_img`.
  2. Build `mixin_key` from a fixed 64-entry permutation, then take the first 32 characters.
  3. Add `wts`, sort the parameters, strip `!'()*` from values, and percent-encode.
  4. `w_rid = md5(query + mixin_key)`.
  - The keys rotate about daily; cache them and refresh on `-352`.

### 3.2 Quality without login

- **Documented** ([videostream_url.md mirror](https://raw.githubusercontent.com/rinnein/bilibili-API-collect/master/docs/video/videostream_url.md)):
  - qn 74/80 (720p60/1080p) need login; 112 and above need VIP;
  - `try_look=1` "can pull 64 and 80 without login";
  - FLV (`fnval=0`) is discontinued; `fnval=1` is MP4 (one file); `fnval=16`/`4048` is DASH.
- **Measured:**
  - `fnval=1` gave **720p MP4**.
  - `fnval=4048` gave DASH at **480p** for one video, and **1080p** for another only with `try_look=1`. I could not find the rule **[unverified]**.
  - HDR, Dolby and 4K are VIP-only.
- **Plan:** request both, and play the higher of MP4 (single file, simplest) and DASH (separate audio, through the host's EDL).

### 3.3 Headers and how mpv gets them

- **CDN behaviour (live probe):**
  - `upos-*-mirrorcosov.bilivideo.com` returned 403 without a `Referer`, and 206 with `Referer: https://www.bilibili.com/`.
  - An Akamai mirror accepted requests without a Referer but rejected curl's User-Agent.
  - **Always send `Referer: https://www.bilibili.com/` and a browser `User-Agent`**, the same pair yt-dlp uses (`bilibili.py L50, L729`).
- **URL lifetime:** URLs carry `deadline`, which was now + 7200 s (2 hours).
- **Risk-control codes:**
  - `-352` (possibly with `v_voucher`, needing a captcha to recover) and `-412` appear on abnormal or repeated requests ([v_voucher.md mirror](https://raw.githubusercontent.com/rinnein/bilibili-API-collect/master/docs/misc/sign/v_voucher.md)).
  - Thresholds and behaviour from mainland Chinese IPs are **[unverified]**.
- **How the headers reach mpv:** the SDK's `ResolvedResource` has `headers?: Record<string,string>` (`SDK:7988-7999`). The host turns them into per-file `http-header-fields` for both streams (`host/src/source_resolvers.rs:340-353`). **No new API is needed for headers.** The existing Bilibili plugin already passes yt-dlp's headers this way (`plugins-repo/plugins/bilibili/main.ts:15-28`, `extractor.ts:27-40`).
- **Thumbnails need no headers.**
  - `https://i0.hdslb.com/...jpg@320w_180h.jpg` returned 200 `image/jpeg`, 16 KB. The same with `.webp` returned WebP, which `ui.image` does not accept; use the `.jpg` form.
  - Rewrite `http://` covers to `https://`. Remote Markdown images and our own fetches should use https.

### 3.4 JS/TS libraries

- **Actively maintained:** all GPL/AGPL, so they cannot go into an Apache-2.0 plugin.
  - `@renmu/bili-api` 2.15.2 ("GPL")
  - `@yukariyukaro/bilibili-api-ts` 17.5.0 (GPL-3.0+)
  - `@seiuna/bilibili-api` (GPL-3.0)
  - `bilicaptain` (AGPL-3.0)
- **Permissive:** `bili-api` (MIT) is stale (2023). `qxr-bilibili-api` (ISC, 2026-08) is based on the archived collection; its quality is **[unverified]**. Source: npm registry.
- **Recommendation:** write the about 150 lines ourselves (WBI, five endpoints, header set) on Node's `fetch` and `crypto`. **No npm dependency.**

### 3.5 Legal and terms of service (factual, not legal advice)

- **The user agreement** (unofficial mirror, effective 2025-05-07, [elawcn](https://www.elawcn.com/agreement/2025/1120/1648.html)):
  - §5.6(3) forbids crawlers or automated scripts without written permission;
  - §8.2 forbids reverse engineering;
  - §8.5 forbids "providing viewing services" without permission.
  - The official page renders client-side, so its text is **[unverified]**.
- **Enforcement:**
  - The third-party client PiliPala received an infringement letter in July 2025 and stopped updates ([IT之家](https://www.ithome.com/0/869/202.htm)).
  - The API collection was taken down in January 2026 (above).
- **YouTube's terms** likewise forbid access by automated means other than its public APIs.
- A plugin that lists, plays and **downloads** from either site carries this risk for the plugin repository (`vestrasys`). Whether to host such plugins is a user decision (workflow §7).

## 4. What our plugin API offers, and what is missing

### 4.1 Already available

- **Sidebar panel.** `plugin.panels.mount("sidebar", file, { title })` (`SDK:4355-4389`). Notes:
  - one panel per plugin; up to 8 sidebar tabs, so both plugins and the playlist fit (`SDK:3881-3895`);
  - `Panel.show/toggle`, and `setInteraction({ focused })` to keep the panel up while the search box has focus (`SDK:3933-3946`);
  - `showMenu` for a row's context menu;
  - the playlist is the precedent (`plugins/playlist/main.ts:147`).
- **Network.** Plugins are full Node 26 processes with "the file system, the network and npm packages" (`SDK:15`, `SDK:223-225`). Capabilities do not limit network access (`SDK:2168`).
- **Playing a listed item.** `plugin.player.open("https://www.youtube.com/watch?v=…")` routes through the plugin's own resolver (`SDK:543`, `SDK:5732-5850`), as both plugins already do (`plugins-repo/plugins/youtube/main.ts:40-45, 54`). The playlist and Open URL also reach the resolver.
- **Separate audio and headers.** `ResolvedMedia { primary, audio?, title }` with per-resource `headers` (`SDK:6515-6533`, `SDK:7988-7999`). The host builds the EDL plus `http-header-fields` (§1).
- **Images.**
  - `plugin.ui.image(path)`: a PNG/JPEG under the package or data directory, as the **whole value** of an `image` property (`SDK:3599-3628`).
  - `plugin.ui.markdown(text, { remoteImages: true })` downloads https images into a cache: 4 MiB per image, 10 s timeout, 4 at a time, 32 MiB cache, 24 h max age. A Markdown document *may* be nested in list rows (`SDK:2246-2258`, `SDK:859`).
- **Downloads without yt-dlp.**
  - `plugin.dialogs.saveFile` exists, and the current plugin uses it (`plugins-repo/plugins/youtube/main.ts:71`).
  - `plugin.tools.run("ffmpeg", …)` needs no capability and uses the **bundled** ffmpeg in packaged players (`SDK:6234-6260`, `host/src/runtime_paths.rs:5-18, 140-165`). It can mux separate DASH audio and video into MKV, replacing yt-dlp plus a user-installed ffmpeg. Fetch with Node, or let ffmpeg read the URLs with `-headers`.
- **Bundling npm dependencies.** The SDK's `package.json` field `"vestra": { "bundle": true }` bundles with the plugin's own esbuild. "A plugin with runtime dependencies must bundle: the package has no `node_modules`" (`SDK:3047-3060`). Built-in precedent: `plugins/nes/package.json` (`jsnes` plus `esbuild`, `bundle: true`). Extra files go through `"assets"`; licenses go in `licenses/` (`SDK:3035-3040`, workshop `plugins/workshop/package.json`).

### 4.2 Gaps, as candidate §4.1 requests (for the plugin authors to file; I have not filed them)

**G1. An image inside a list row (both plugins).** API maintainer.

- **Problem:** "an image from ui.image, only as the **whole** value of an `image` property (nested in an array or object it is `invalid`)" (`SDK:859`). A results list of `[ { title, duration, channel, thumb } ]` cannot carry thumbnails. The built-in store has the same limit: its list rows have no icons, and only the detail pane has `d-icon` (`plugins/store/ui/store.slint:169, 338`).
- **Desired usage:** `panel.set({ rows: results.map(r => ({ title: r.title, thumb: plugin.ui.image(r.file) })) })` with `in property <[Row]> rows` and `struct Row { title: string, thumb: image }`.
- **Acceptance:** 50 rows with thumbnails scroll smoothly in the sidebar; replacing rows releases the old images.
- **Workarounds that exist today, all poor:**
  - (a) A fixed number of `image` properties (`thumb0…thumb19`) and hand-written rows.
  - (b) One `MarkdownRef` per row with a remote image: little layout control.
  - (c) Combine all thumbnails into one sprite with the bundled ffmpeg (`tools.run`, `xstack`/`tile`) and clip per row with Slint `source-clip-*` ([Slint Image docs](https://docs.slint.dev/latest/docs/slint/reference/elements/image/)). Clever, but it costs an ffmpeg run per page **[unverified]** that source-clip works on runtime images.
- **Related:** runtime images are decoded on the UI thread (about 0.1 s for 4096², `doc/process/rounds/2026-10-features-r3b.md:269` in an older worktree copy). 320×180 thumbnails are cheap, but the host should decode them lazily for visible rows.

**G2. Stream URLs that expire during playback.** API maintainer; **[unverified whether this is a real gap]**.

- Bilibili URLs expire after 2 h (§3.3); YouTube URLs also carry an expiry.
- If the user pauses longer than that, or a long video outlives the URL, the next HTTP range request fails with 403.
- I found no documented way for the host to call the resolver again on a network failure and resume at the same position. The SDK documents `resolve` only for opening (`SDK:5822-5850`).
- **Request:** "On an HTTP 403/410 from a resolved source, re-run the resolver and reopen at the current position, at most once per N minutes". Or document that the plugin must handle it: `player.state` error, then `player.open` again, then `seek`.
- **Acceptance:** play a Bilibili video, pause 2 h 5 min, resume; playback continues.

**G3. A per-hosted-plugin npm dependency setup in the plugins repository.** Infrastructure maintainer, not the API.

- Hosted plugins have no `package.json`. The repository has one root `package.json` with only dev dependencies (`plugins-repo/package.json`).
- The rules say a hosted plugin directory holds `manifest.json`, sources, tests, README and CHANGELOG (`plugins-repo/CONTRIBUTING.md:50-52`), and "Code imports only `@vestra-player/sdk`" (`CONTRIBUTING.md:84-86`).
  - `scripts/check-boundaries.mjs:61-73` does not reject bare package names. Only the CONTRIBUTING rule does.
- The YouTube plugin's `dependencies.lock.json` currently says "no npm production dependencies".
- **Request:**
  - allow `plugins/<name>/package.json` plus a lockfile with `"vestra": { "bundle": true }`;
  - run `npm ci` for it in CI;
  - check dependency licenses, and put third-party license texts in `licenses/`;
  - review the bundled output size.
- **Acceptance:** `npm run plugin -- pack youtube` produces a `.vxp` with youtubei.js bundled, and CI checks the license list.

**G4. Site storyboards for seek thumbnails.** Optional; see the seek-thumbnails report §3.7. Add an optional `preview` sheet field on `ResolvedMedia`, so `vestra.playback` can show Bilibili `videoshot` and YouTube storyboard sprites without the plugins talking to each other.

**Not gaps** (checked):
- Headers for both streams (§3.3).
- Separate audio (§1).
- Network access and running the evaluator in `worker_threads` (pure Node).
- ffmpeg for downloads (bundled).
- Sidebar tabs (8).
- The text field in a panel (`setInteraction({ focused })`).

## 5. Proposed design (per plugin)

### 5.1 Renaming

- **Keep the ids** `zysilm.youtube` and `zysilm.bilibili`. A changed id is a new plugin in the store, and its history is lost (`plugins-repo/CONTRIBUTING.md:48-49`).
- **Change only `name`** to "VestraTube" / "VestraBili", bump the minor version, and add a CHANGELOG entry.
- Update `description` and the README to remove "needs yt-dlp" (`plugins-repo/plugins/youtube/manifest.json:18`, `bilibili/manifest.json:16`).
- **New capability:** `ui.mount`. Keep `ui.dialog` for the save dialog.

### 5.2 Panel layout (sidebar tab)

```text
┌ VestraTube ─────────────────────────┐
│ [ Search YouTube…            ] (⌕)  │  text field; Enter searches; ↓ shows suggestions
│ ( Search results | Home )           │  Bili: ( 搜索 | 热门 | 排行榜 )
├─────────────────────────────────────┤
│ ┌────────┐ Title, two lines max…    │  row: 120×68 thumbnail (16:9) with the
│ │ thumb  │ Channel · 1.2M views     │  duration badge drawn bottom-right in Slint;
│ │  12:34 │                          │  click: play; right-click: Play / Add to
│ └────────┘                          │  playlist / Copy link / Download…
│ …                                   │
│        [ Load more ]                │  next page (continuation / pn+1)
├─────────────────────────────────────┤
│ [ Open URL… ]  [ Download… ]        │  bottom bar: today's two flows
└─────────────────────────────────────┘
```

- **"Open URL…"** opens the existing tool window (`youtube.slint` / `dialog.slint`). Its play logic is unchanged; it goes through the resolver.
- **"Download…"** downloads the selected row, or asks for a URL if no row is selected. Then the save dialog, a progress line in the bar, and Cancel.
- **The "Home" tab:**
  - YouTube: shows `getHomeFeed()` if a guest session returns items, otherwise fixed category searches (§2.2).
  - Bilibili: `popular` and `ranking/v2`, which work anonymously.

### 5.3 Data flow

- **Search:**
  1. Panel `search(text)`.
  2. Plugin: cancel the previous request (`AbortController`), then fetch. YouTube: youtubei.js `search`. Bilibili: signed `wbi/search/type` with a random `buvid3`.
  3. Map results to rows `{ id, title, channel, duration, views, url }`.
  4. Bind the rows (they fit the 4096-rows and 1 MiB text limits, `SDK:2223-2232`).
  5. Thumbnails stream in (§5.4).
- **Play:**
  1. Row `play(id)` calls `player.open(pageUrl)`.
  2. The plugin's own resolver builds `ResolvedMedia`:
     - YouTube: VISIONOS formats, best H.264/VP9 ≤ 1080p plus audio, the client's User-Agent.
     - Bilibili: `view`/`pagelist` to get the `cid`, then `playurl` (`fnval=1` and `4048`, `try_look=1`), then MP4 or DASH, with Referer and User-Agent.
  3. The host plays it. Status and errors go to the panel's status line.
- **Download:**
  1. The save dialog.
  2. Resolve as above.
  3. Fetch the video and audio with Node streams, with progress reported from Content-Length.
  4. `tools.run("ffmpeg", ["-i", v, "-i", a, "-c", "copy", out.mkv])` into a staging directory next to the target, then rename. This is the same safety pattern as today (`plugins-repo/plugins/youtube/service.ts:108-134`).
  - Note: `tools.run`'s default `maxOutput` is 16 MiB of *stdout*, which does not apply to the output file.
- **Shared code per plugin:** one `client.ts` (site API), one `resolver.ts`, `panel.ts`, `download.ts`. YouTube adds an `evaluator-worker.ts` for the deciphering JS.

### 5.4 Thumbnail caching

- **Path:** `dataDirectory/thumbs/<sha1(url)>.jpg`.
  - YouTube URL: `i.ytimg.com/vi/<id>/mqdefault.jpg`.
  - Bilibili URL: `<pic>@320w_180h.jpg`.
- **Fetching:** at most 4 at a time, a 10 s timeout and a 1 MB limit, mirroring the Markdown limits (`SDK:2249-2257`). The rows show a grey placeholder until the file exists.
- **Eviction:** an LRU index in `plugin.storage` or a JSON store, capped at about 50 MB, with entries older than 7 days removed.
- **Display depends on G1.** Until G1 lands, use workaround (a) for the visible page only (20 rows): `thumb0`…`thumb19` properties.

### 5.5 Failure and offline states (one status line, `UI-TOOL-05`-style, as today)

| State | Detection | Shown |
|---|---|---|
| Offline / DNS failure | fetch `TypeError` / `ENOTFOUND` | "No network connection." plus a Retry button; cached thumbnails still show |
| Bilibili risk control | `code -352` / `-412` / HTTP 412 | "Bilibili is limiting requests; try again later." with backoff; no automatic retry loop |
| YouTube bot check | "Sign in to confirm…" / `LOGIN_REQUIRED` | "YouTube asked to confirm you are not a bot (this network is flagged). Try another network." |
| Unplayable (members, region, DRM, made-for-kids on all clients) | playability status | the site's reason, one line |
| Extraction broke (site change) | decipher/parse error | "Could not read this video. The plugin may need an update." plus the plugin version |
| Empty home feed | 0 items | "Nothing to show here; try a search." |
| URL expired mid-play | host error after a long pause | depends on G2 |

### 5.6 Effort estimate

| Work | Owner | Estimate |
|---|---|---|
| Bilibili client (WBI, search, popular, ranking, view, playurl), resolver without yt-dlp, tests with recorded fixtures | Bilibili author | 1.5–2 days |
| Bilibili panel (Slint list, tabs, bottom bar), thumbnail cache, download via ffmpeg | Bilibili author | 1.5–2 days |
| YouTube: youtubei.js integration, evaluator worker, client fallback chain, resolver | YouTube author | 2–3 days |
| YouTube panel, thumbnail cache, download via ffmpeg | YouTube author | 1.5–2 days |
| G1: images in model rows (host `ui_values`, SDK types, TestHost, docs) | API maintainer | 2–3 days |
| G2: re-resolve on expiry (if confirmed) | API maintainer | 1–2 days |
| G3: per-plugin npm dependencies, licenses in the plugins-repo CI | Infrastructure maintainer | 1 day |
| GUI tests, Windows and macOS | Coordinator | 1 day |

**Total:** about 6.5–9 plugin-author days, about 3–5 maintainer days and 1 infrastructure day. The coordinator also has to plan ongoing YouTube upkeep: dependency bumps after site changes, possibly every few weeks.

### 5.7 Main risks

1. **YouTube extraction breaking** (§2.3-2.4). It is outside our control. Keep the optional installed-yt-dlp fallback.
2. **Terms of service and legal exposure** for both sites, including Bilibili's recent letters (§3.5). This needs a user ruling before hosting.
3. **Bilibili risk control** for heavy users or some regions **[unverified from CN IPs]**.
4. **Bundle size and review effort** of youtubei.js in a hosted plugin (G3).

---

## User ruling (2026-10-26)

- **VestraTube** (`zysilm.youtube`): build the sidebar (search-first list with thumbnail, title, duration; direct play without login; URL play and download from a button at the sidebar's bottom) on a bundled open-source library: **youtubei.js** as recommended above. No yt-dlp install. If the upstream library itself is blocked or broken by YouTube, that is accepted. We follow the library's fixes; we don't build our own extractor.
- **VestraBili** (`zysilm.bilibili`): **keep the current "paste a link to play" scope.** No browsing sidebar for now (Bilibili's legal pressure on third-party clients). Only the rename to VestraBili and English-by-default.
- The display names change; the ids `zysilm.youtube` / `zysilm.bilibili` stay (coordinator decision: changing the id would orphan existing installs).

## Acceptance (coordinator, Windows, 2026-10-27; VestraTube 0.4.1 from `zysilm.youtube-0.4.1.vxp`)

User checks on 0.4.0 (2026-10-26) found: nothing shown before a search; selected search text unreadable in the dark palette; a double-click on a not-yet-selected row's text only selected it; the search box text too small. 0.4.1 fixes all four (the row cause was the host's `ToolTipArea` popup swallowing the next press, plus full-model rebuilds on each selection; the plugin now sends plain rows with a separate image list and no tooltip) and adds infinite scroll; the user ruled that with no reliable logged-out recommendation source the empty state is only "Search and press Enter".

Scripted recheck of 0.4.1: empty state shows the hint and sends no request; "lofi" + Enter lists results with thumbnails, duration/LIVE badges and channel names; Ctrl+A in the search box gives a readable blue selection; wheel-scrolling keeps loading pages without a "More results" button; a double-click on a row's title text after hovering 1.2 s plays it at once (1080p30 stream, 30/30 fps). Not rechecked: the failure/Retry row (needs the network cut), the download button.
