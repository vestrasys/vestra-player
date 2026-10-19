# Generic source resolver increment — 2026-10-19

Wire remains `vestra.experimental/0.1`. Trusted JS providers register extraction handlers; Rust has no site rules, executable downloader or plugin-ID branches. Manifest capability: `source.resolve`.

```ts
const registration = await host.sourceResolvers.register(
  { id: "provider", schemes: ["http", "https"], hosts: ["media.example"] },
  async ({ url, signal }) => ({
    kind: "media",
    originalUrl: url,
    title: "Optional title",
    primary: { url: videoUrl, headers },
    audio: { url: audioUrl, headers }, // omit for a combined resource
  }),
);
// Disposal aborts active handler signals and unregisters the declaration.
await registration.dispose();
```

Declarations contain only serializable exact host names and HTTP(S) schemes. Path recognition and extraction stay in the handler. There are at most 16 registrations, 16 hosts per declaration and 32 pending resolutions. Overlapping host/scheme pairs are rejected with `conflict`; registration IDs are scoped to the owner. Owner shutdown removes its registrations.

All callers use the original public route:

```ts
const operation = await host.source.submit(
  { kind: "open", source: originalUrl, startPaused: false, expected: stamp },
  operationId,
);
// A resolving request is pending with the existing SourceTicket.
// Recover an uncertain reply with source.operation(theSameOperationId).
```

Matching declarations route through resolution; unmatched URLs retain the existing direct-media route. No claim is made that an arbitrary page works without its provider. Admission uses the existing source operation book and CAS before dispatching extraction. The backend accepts a descriptor only while the original stamp/cycle is current. A replacement intent, owner loss, disposal or the 30-second deadline fails pending work and sends AbortSignal cancellation. An old descriptor cannot replace a later source. `source.state().source` remains the original URL; descriptor URLs and headers are not copied into public source state. Exact operation replay does not invoke extraction twice.

Descriptors accept bounded HTTP(S) resource URLs, at most 32 headers and 16KiB total header text per resource, a 1KiB optional title, and matching originalUrl. Header names are normalized case-insensitively; duplicate names, newline injection and oversized data are rejected. Split video/audio is passed as a paired mpv load with file-local HTTP headers. The current backend requires identical normalized header maps for the two resources; different maps fail explicitly instead of dropping a map. This is a finite backend restriction, not a guarantee of arbitrary per-resource transport options. [mpv reference: file-local load options and HTTP headers](https://mpv.io/manual/stable/).

Evidence: `/tmp/vestra-current-candidates-NwkA7t/result.json` has ten actual public-RPC assertions passing: declaration conflict, pending ticket, split-stream ready, original public identity, both HTTP resource requests with required headers, selected AAC track, exact replay, later-source CAS, handler abort, and explicit different-header refusal. The fixture uses independently served video-only MP4/audio-only M4A and enforces its HTTP header before serving ranges. This was headless with silent output; it does not establish real-site playback or speaker output. Host SIGTERM cleanup was awaited. Build, strict SDK checking, formatting and the two affected pure Rust tests passed; Clippy status is recorded separately in the raw log.
