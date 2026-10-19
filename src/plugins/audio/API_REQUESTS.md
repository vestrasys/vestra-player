# API usage and remaining acceptance

No SDK expansion is requested. This product uses commands, source.state/events, processing.replace/operation and independent ui.open/set/show/close windows, with granular commands/playback.read/events/processing.audio/ui.window capabilities.

Ownership is host authenticated. One resourceId `workbench`, audio stage, monotonic revision, expectedGeneration, typed equalizer/treble/dynaudnorm nodes and offsetMs implement the workbench. There is no legacy af string, sibling plugin dependency or global volume/mute operation.

Transport failures query the same operation token. Pending/unknown/unknownExpired remain blocked until explicit recovery; no token is reused with a changed payload. Definitive failures in the current epoch expose retry with a new revision. Definitive failures from a previous track/source epoch do not block the queued current neutral settings. An old unknown operation still blocks submission until terminal, because track changes alone do not prove backend completion.

Required native acceptance remains: Slint compilation/window activation; callback and menu state; actual typed backend effects and delay sign; source/track transition behavior; window close/reopen; host cleanup on disable. This author has not run Host, Cargo, GUI or actual sound.
