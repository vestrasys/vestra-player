# Current API usage and limits

No new API requested. Uses `commands`, `playback.read`, `events`, `processing.video`, `ui.window`; all processing is a typed owned `picture` resource with eq and optional three-step gblur.

No processing-state subscription exists in the SDK; source-state changes and explicit check/retry refresh it. Pending/unknown operation results block further replacements; unknownExpired without a matching resource revision explicitly requires owner reload for safe cleanup. No positive video-stream capability exists in source state, so unsupported processing is determined by the actual backend rejection and shown truthfully.

Backend acceptance is not a rendered-frame fence. Actual paused-frame visibility and decoder transition persistence remain native/media verification boundaries.
