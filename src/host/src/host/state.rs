//! Source, window, picture-in-picture, layout and backdrop state, the
//! title, power and always-on-top.
use super::*;
use crate::playback::renderer::GpuBackend as _;

/// What a `layout.set` changes: the preference, floating, the current tab.
#[derive(Debug, PartialEq)]
pub(crate) struct LayoutChange {
    pub(crate) visible: Option<bool>,
    pub(crate) detached: Option<bool>,
    /// The owner of the panel whose tab becomes current.
    pub(crate) active: Option<u64>,
}

/// Read `layout.set`'s `visible`, `detached` and `active` (a plugin id among
/// the slot's `panels`: owner, manifest id) for `slot`.
pub(crate) fn layout_change(
    params: &Value,
    slot: composition::Slot,
    panels: &[(u64, String)],
) -> std::result::Result<LayoutChange, RpcError> {
    let flag = |key: &str| -> std::result::Result<Option<bool>, RpcError> {
        match params.get(key).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(value) => value
                .as_bool()
                .map(Some)
                .ok_or_else(|| RpcError::invalid(format!("{key} boolean required"))),
        }
    };
    let (visible, detached) = (flag("visible")?, flag("detached")?);
    if detached.is_some() && slot == composition::Slot::Bottom {
        return Err(RpcError::invalid("the bottom bar cannot be detached"));
    }
    let active = match params.get("active").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::String(id)) => Some(
            panels
                .iter()
                .find(|(_, manifest)| manifest == id)
                .map(|(owner, _)| *owner)
                .ok_or_else(|| {
                    RpcError::new("missing", format!("{id} has no panel at this position"))
                })?,
        ),
        Some(_) => return Err(RpcError::invalid("active requires a plugin id")),
    };
    Ok(LayoutChange {
        visible,
        detached,
        active,
    })
}

/// `WindowState.maximized` from the main window's facts (`window_tick`):
/// never while fullscreen, false before the window was measured.
fn reported_maximized(facts: &Value) -> bool {
    facts["maximized"] == true && facts["fullscreen"] != true
}

pub(crate) fn policy_slot(slot: composition::Slot) -> Option<window_state::Slot> {
    match slot {
        composition::Slot::Bottom => Some(window_state::Slot::Bottom),
        composition::Slot::Sidebar => Some(window_state::Slot::Sidebar),
        composition::Slot::Backdrop => None,
    }
}

impl Host {
    /// Merge the sequence provider's flags into a source state (C6).
    pub(crate) fn patch_source(&self, state: &mut Value) {
        if state.is_object() {
            let (next, previous) = self
                .sequence
                .map(|(_, next, previous)| (next, previous))
                .unwrap_or((false, false));
            state["sequence"] = json!({"next":next,"previous":previous});
        }
    }
    /// Re-send the last source state with the current sequence flags.
    pub(crate) fn rebroadcast_source(&mut self) {
        if self.last_source.is_object() {
            let mut state = self.last_source.clone();
            self.patch_source(&mut state);
            self.last_source = state.clone();
            self.broadcast("source.state", state);
            self.core_action_states();
        }
    }
    /// R-U3: follow the source into `no-video`; an empty source also drops
    /// the last video frame, so the video area shows the backdrop or plain colour.
    /// A mounted backdrop shows audio files itself: mpv stops picking cover
    /// art as a video track while one is mounted (from the next file opened).
    pub(crate) fn backdrop_cover_art(&mut self) {
        let mounted = self
            .mounts
            .values()
            .any(|m| m.spec.slot == composition::Slot::Backdrop);
        if mounted != self.cover_art_off {
            self.cover_art_off = mounted;
            let _ = self.backend.commands.send(Command {
                reply: Reply {
                    owner: bindings::HOST,
                    id: 0,
                },
                method: "host.coverArt".into(),
                params: json!({ "off": mounted }),
            });
        }
    }
    pub(crate) fn backdrop_source(&mut self) {
        self.video_absent = composition::video_absent(self.video_absent, &self.last_source);
        let next = composition::no_video(self.no_video, &self.last_source);
        let empty = self.last_source["phase"] == "empty";
        let Some(shell) = &self.shell else {
            self.no_video = next;
            return;
        };
        // GPU frames are not kept in `last_image`: clear once per empty phase.
        let gpu = self
            .backend
            .render
            .gpu()
            .filter(|_| empty && !self.video_cleared);
        self.video_cleared = empty;
        if empty && self.last_image.take().is_some() || gpu.is_some() {
            let _ = shell.set_property("video-frame", SValue::Image(Default::default()));
            // The next source starts from its own first frame, not this one's last.
            if let Some(gpu) = gpu {
                gpu.reshow();
            }
        }
        if next != self.no_video {
            self.no_video = next;
            let _ = shell.set_property("no-video", next.into());
        }
    }
    /// C9: "<source name> - Vestra Player", or "Vestra Player" without a source.
    pub(crate) fn update_title(&mut self) {
        let source = &self.last_source;
        let name = match source["source"].as_str() {
            None => None,
            Some(_) if source["kind"] == "producer" => None,
            Some(text) if text.contains("://") => Some(
                source["title"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .unwrap_or(text)
                    .to_owned(),
            ),
            Some(text) => Some(
                Path::new(text)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| text.to_owned()),
            ),
        };
        let title = name.map_or_else(
            || "Vestra Player".to_owned(),
            |name| format!("{name} - Vestra Player"),
        );
        if title != self.window_title {
            self.window_title = title;
            if let Some(shell) = &self.shell {
                let _ = shell.set_property(
                    "window-title",
                    SValue::String(self.window_title.as_str().into()),
                );
            }
        }
    }
    /// Keep the sleep assertion in line with what is playing: ready and not
    /// paused (media or stream), video when a frame arrived within a second.
    pub(crate) fn power_tick(&mut self) {
        let source = &self.last_source;
        let playback = power::Playback {
            playing: source["phase"] == "ready" && source["paused"] == false,
            video: self
                .last_frame
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1)),
        };
        if let Some(error) = self.power.update(playback) {
            diag!("host.error power {error}");
        }
    }
    /// `window.setAlwaysOnTop`: every ordinary window of the application at
    /// the floating level, applied again when windows open or close.
    pub(crate) fn apply_on_top(&mut self) {
        if self.headless {
            return;
        }
        let want = self.on_top.get();
        let count = platform::window_count();
        if self.on_top_applied == Some((want, count)) {
            return;
        }
        match platform::set_floating(want) {
            Ok(count) => self.on_top_applied = Some((want, count)),
            Err(error) => {
                diag!("host.error window level: {error}");
                self.on_top_applied = Some((want, count));
            }
        }
    }
    /// `{ fullscreen, maximized, videoArea, pictureInPicture }` of the main
    /// window (zero without one; `maximized` is false while fullscreen),
    /// with the palette and the file resolvers' extensions
    /// (`plugin.resolvers.fileTypes`).
    pub(crate) fn window_state(&self) -> Value {
        let facts = &self.window_facts;
        json!({"fullscreen":facts["fullscreen"] == true,
            "maximized":reported_maximized(facts),
            "videoArea":{"width":facts["videoArea"]["width"].as_f64().unwrap_or(0.),
                "height":facts["videoArea"]["height"].as_f64().unwrap_or(0.)},
            "pictureInPicture":self.pip_state(),
            "appearance":self.theme(),"fileTypes":self.file_types,
            "language":self.language_preference().unwrap_or_else(|| i18n::SYSTEM.into()),
            "locale":i18n::locale()})
    }
    /// `WindowState.pictureInPicture`: offered here, and the window showing.
    pub(crate) fn pip_state(&self) -> Value {
        let facts = self.pip_facts();
        json!({"supported":facts.window && facts.supported,"active":self.pip.active()})
    }
    /// `window.setVideoArea { width, height }`: resize the window so the video
    /// column gets that size (bars added, fitted to the screen). Answered when
    /// the layout shows it, or after a second.
    pub(crate) fn set_video_area(
        &mut self,
        reply: &Reply,
        params: &Value,
    ) -> std::result::Result<(), RpcError> {
        let (width, height) = (
            params["width"].as_f64().unwrap_or(f64::NAN),
            params["height"].as_f64().unwrap_or(f64::NAN),
        );
        if !window_control::valid_size(width, height) {
            return Err(RpcError::invalid("width and height must be 1..16384"));
        }
        let unavailable = |message: &str| RpcError::new("unavailable", message);
        let shell = self
            .shell
            .as_ref()
            .ok_or_else(|| unavailable("the video area needs a native window"))?;
        if shell.window().is_fullscreen() {
            return Err(unavailable(&i18n::tr("Not available in fullscreen")));
        }
        let id = self
            .native_window
            .ok_or_else(|| unavailable("the native window is not known"))?;
        let screen = platform::screen_for(id).map_err(|e| unavailable(e.0))?;
        // A size asked for while maximized is the size of a normal window:
        // on Windows, restore it first, then resize it, as MPC-HC's View ▸
        // Zoom does (`ZoomVideoWindow` restores a zoomed window before sizing
        // it), and measure the restored frame. Not on macOS: `zoom:` animates
        // there and would fight the content size set right after, so a zoomed
        // window is resized directly, as before 0.6.3.
        // Automatic sizing leaves a maximized window alone on every system
        // (`WindowState.maximized`, the Window plugin's G-02).
        let screen = if cfg!(target_os = "windows") && shell.window().is_maximized() {
            shell.window().set_maximized(false);
            platform::screen_for(id).map_err(|e| unavailable(e.0))?
        } else {
            screen
        };
        view::settle_layout(shell.window());
        let number = |name: &str| match shell.get_property(name) {
            Ok(SValue::Number(n)) if n.is_finite() => n,
            _ => 0.,
        };
        let sidebar = &self.window_facts["slots"]["sidebar"];
        let docked = sidebar["preferred"] == true && sidebar["detached"] == false;
        let frame = window_control::Frame {
            extra_height: (number("logical-height") - number("video-height")).max(0.),
            extra_width: if docked {
                self.policy.sidebar_user_width() + window_state::SPLITTER_WIDTH
            } else {
                0.
            },
            max_content: platform::max_content(&screen),
        };
        let fit = window_control::fit(width, height, frame);
        platform::resize_content(id, fit.content.0, fit.content.1).map_err(|e| unavailable(e.0))?;
        self.area_replies.push(AreaReply {
            reply: reply.clone(),
            video: fit.video,
            result: json!({"width":fit.video.0,"height":fit.video.1,"clamped":fit.clamped}),
            deadline: self.now_ms() + 1000,
        });
        Ok(())
    }
    pub(crate) fn answer_video_area(&mut self, video: (f64, f64), now: u64) {
        let (done, waiting): (Vec<_>, Vec<_>) = self.area_replies.drain(..).partition(|pending| {
            now >= pending.deadline
                || ((video.0 - pending.video.0).abs() <= 1.
                    && (video.1 - pending.video.1).abs() <= 1.)
        });
        self.area_replies = waiting;
        for pending in done {
            self.respond(pending.reply, Ok(pending.result));
        }
    }
    pub(crate) fn show_info_panels(&self) {
        if let Some(shell) = &self.shell
            && let Ok(value) =
                ui_values::from_json(&json!(self.info_panels.values().collect::<Vec<_>>()))
        {
            let _ = shell.set_property("info-panels", value);
        }
    }
    /// Current layout of both positions, or `pending` until the shell is measured.
    pub(crate) fn layout_state(&self) -> std::result::Result<Value, RpcError> {
        if self.window_facts["shellGeneration"].as_u64() != Some(self.shell_generation) {
            return Err(RpcError::new(
                "pending",
                "current shell layout is not yet confirmed",
            ));
        }
        // `panels`: the slot's panels in tab order (sidebar tabs); `owner`
        // is the current one's.
        let bottom = self
            .mounts
            .values()
            .filter(|m| m.spec.slot == composition::Slot::Bottom)
            .map(|m| m.spec.owner)
            .collect::<Vec<_>>();
        let sidebar = self.policy.sidebar_panels();
        let slot = |name: &str| {
            let slot = &self.window_facts["slots"][name];
            if slot.is_null() {
                return Value::Null;
            }
            let owners = if name == "sidebar" { &sidebar } else { &bottom };
            let panels = owners
                .iter()
                .filter_map(|owner| self.mounts.get(owner))
                .map(|m| json!({"owner":m.spec.owner,"plugin":m.manifest,"title":m.title}))
                .collect::<Vec<_>>();
            json!({"owner":slot["owner"],"preferred":slot["preferred"],"effective":slot["effective"],
                "reason":slot["reason"],"detached":slot["detached"],"panels":panels})
        };
        Ok(
            json!({"bottom":slot("bottom"),"sidebar":slot("sidebar"),"backdrop":self.backdrop_state()}),
        )
    }
    /// `panels.backdrop.state`: `{ owner, effective }`, null without a backdrop.
    pub(crate) fn backdrop_state(&self) -> Value {
        self.mounts
            .values()
            .find(|m| m.spec.slot == composition::Slot::Backdrop)
            .map_or(
                Value::Null,
                |m| json!({"owner":m.spec.owner,"effective":self.no_video}),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `window.state` reports `maximized` next to `fullscreen`, false in
    /// fullscreen and before the first measurement; a maximize or restore
    /// alone changes the facts, so `window_tick` broadcasts it.
    #[test]
    fn maximized_is_reported_and_a_change_of_it_alone_is_a_change() {
        let facts = |fullscreen: bool, maximized: bool| json!({"fullscreen":fullscreen,"maximized":maximized,"videoArea":{"width":800.,"height":450.}});
        assert!(reported_maximized(&facts(false, true)));
        assert!(!reported_maximized(&facts(false, false)));
        assert!(
            !reported_maximized(&facts(true, true)),
            "never in fullscreen"
        );
        assert!(!reported_maximized(&Value::Null), "not measured yet");
        assert_ne!(facts(false, true), facts(false, false));
    }

    #[test]
    fn layout_set_reads_the_current_tab_by_plugin_id() {
        use composition::Slot;
        let panels = [
            (4, "vestra.playlist".to_string()),
            (9, "examples.panel-menu".into()),
        ];
        assert_eq!(
            layout_change(
                &json!({"slot":"sidebar","active":"examples.panel-menu","visible":true}),
                Slot::Sidebar,
                &panels
            )
            .unwrap(),
            LayoutChange {
                visible: Some(true),
                detached: None,
                active: Some(9)
            }
        );
        assert_eq!(
            layout_change(
                &json!({"active":null,"detached":false}),
                Slot::Sidebar,
                &panels
            )
            .unwrap(),
            LayoutChange {
                visible: None,
                detached: Some(false),
                active: None
            }
        );
        let code = |params: Value, slot| layout_change(&params, slot, &panels).unwrap_err().code;
        assert_eq!(
            code(json!({"active":"other.plugin"}), Slot::Sidebar),
            "missing"
        );
        assert_eq!(code(json!({"active":7}), Slot::Sidebar), "invalid");
        assert_eq!(code(json!({"visible":"yes"}), Slot::Sidebar), "invalid");
        assert_eq!(code(json!({"detached":true}), Slot::Bottom), "invalid");
    }
}
