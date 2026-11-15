//! Source, window, picture-in-picture, layout and backdrop state, the
//! title, power and always-on-top.
use super::messages::{UiCommand, UiReport, UiSnapshot, WindowOp};
use super::*;
use crate::playback::renderer::GpuBackend as _;

/// The panel policy's tabs (`window_state::Policy`) as the interface
/// reports them with its window facts (rule R9; plan step S2c(5)): the
/// bottom bar's panel and each column's panels in tab order. A column
/// holds at least one panel or does not exist, and its current tab is one
/// of its panels. The core keeps the last one reported
/// (`Core::panel_tabs`) and judges from it what the policy would refuse
/// (`window_state::Error::MissingOwner`, `Docked`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PanelTabs {
    pub(crate) bottom: Option<u64>,
    pub(crate) sidebar: Vec<u64>,
    pub(crate) adjust: Vec<u64>,
}

impl PanelTabs {
    /// A column's panels (`None`: the bottom bar, which has no column).
    fn column(&self, slot: window_state::Slot) -> Option<&[u64]> {
        match slot {
            window_state::Slot::Bottom => None,
            window_state::Slot::Sidebar => Some(&self.sidebar),
            window_state::Slot::Adjust => Some(&self.adjust),
        }
    }
    /// The column holding `owner`'s panel (`Policy::tabs_mut`).
    fn column_of(&self, owner: u64) -> Option<window_state::Slot> {
        [window_state::Slot::Sidebar, window_state::Slot::Adjust]
            .into_iter()
            .find(|&slot| self.column(slot).is_some_and(|tabs| tabs.contains(&owner)))
    }
    /// The column of `slot` exists (has a panel, so a current tab).
    fn has_column(&self, slot: window_state::Slot) -> bool {
        self.column(slot).is_some_and(|tabs| !tabs.is_empty())
    }
    /// `Policy::set_locks`, `set_preferred` and `switch` for `owner` in
    /// `slot`: the bottom bar's panel, or one of the column's.
    pub(crate) fn holds(
        &self,
        slot: window_state::Slot,
        owner: u64,
    ) -> std::result::Result<(), window_state::Error> {
        let held = match self.column(slot) {
            None => self.bottom == Some(owner),
            Some(tabs) => tabs.contains(&owner),
        };
        held.then_some(()).ok_or(window_state::Error::MissingOwner)
    }
    /// `Policy::set_active`: a panel of either column.
    fn set_active(&self, owner: u64) -> std::result::Result<(), window_state::Error> {
        self.column_of(owner)
            .map(|_| ())
            .ok_or(window_state::Error::MissingOwner)
    }
    /// `Policy::set_detached` for a panel of `column` (`None`: of none).
    fn set_detached(
        column: Option<window_state::Slot>,
    ) -> std::result::Result<(), window_state::Error> {
        match column {
            None => Err(window_state::Error::MissingOwner),
            Some(window_state::Slot::Adjust) => Err(window_state::Error::Docked),
            Some(_) => Ok(()),
        }
    }
    /// `layout.set`'s steps on the policy, in its order (the current tab,
    /// floating, the preference; `Host::layout_apply`), for `change` in
    /// `slot` whose first panel is `first`: the panel the steps go
    /// through is the column's current tab, else `first`. `Err` when a
    /// step is refused before any step took effect: nothing changes, and
    /// the core refuses at once. A step refused after an earlier one took
    /// effect is the interface's to report, after that earlier step (as
    /// today), so that case is `Ok`.
    pub(crate) fn judge_layout(
        &self,
        slot: composition::Slot,
        policy_slot: window_state::Slot,
        change: &LayoutChange,
        first: u64,
    ) -> std::result::Result<(), window_state::Error> {
        let current = slot.tabbed() && self.has_column(policy_slot);
        let mut changed = false;
        if let Some(active) = change.active
            && slot.tabbed()
        {
            self.set_active(active)?;
            changed = true;
        }
        if change.detached.is_some() {
            let column = if current {
                Some(policy_slot)
            } else {
                self.column_of(first)
            };
            match Self::set_detached(column) {
                Err(error) if !changed => return Err(error),
                Err(_) => return Ok(()),
                Ok(()) => changed = true,
            }
        }
        if change.visible.is_some() && !current && !changed {
            self.holds(policy_slot, first)?;
        }
        Ok(())
    }
}

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
    if detached == Some(true) && slot == composition::Slot::Adjust {
        return Err(RpcError::invalid("the adjust column cannot be detached"));
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
        composition::Slot::Adjust => Some(window_state::Slot::Adjust),
        composition::Slot::Backdrop => None,
    }
}

impl Host {
    /// Re-send the last source state with the current sequence flags.
    pub(crate) fn rebroadcast_source(&mut self) {
        if self.core.last_source.is_object() {
            let mut state = self.core.last_source.clone();
            self.core.patch_source(&mut state);
            self.core.last_source = state.clone();
            self.source_shown();
            self.broadcast("source.state", state);
            self.core.core_action_states();
        }
    }
    /// The core's last `source.state` reaches the interface's copy
    /// (`UiSnapshot::LastSource`; plan step S2c(7)), in place, right where
    /// the core replaced its own: what the interface reads from the source
    /// from here on is what the core has.
    pub(crate) fn source_shown(&mut self) {
        let source = Arc::new(self.core.last_source.clone());
        self.ui_command(UiCommand::Snapshot(UiSnapshot::LastSource(source)));
    }
    /// R-U3: follow the source into `no-video`; an empty source also drops
    /// the last video frame, so the video area shows the backdrop or plain
    /// colour (the interface's copy of the source, `Host::source`).
    pub(crate) fn backdrop_source(&mut self) {
        self.video_absent = composition::video_absent(self.video_absent, &self.source);
        let next = composition::no_video(self.no_video, &self.source);
        let empty = self.source["phase"] == "empty";
        let Some(shell) = &self.shell else {
            self.no_video = next;
            return;
        };
        // GPU frames are not kept in `last_image`: clear once per empty phase.
        let gpu = self
            .core
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
        let source = &self.source;
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
        let source = &self.core.last_source;
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
    /// the floating level, applied again when windows open or close; not
    /// while a system dialog is open (`window_control::floating`), and
    /// back when it closes (the next tick).
    pub(crate) fn apply_on_top(&mut self) {
        if self.core.headless {
            return;
        }
        self.sync_window_settings();
        let want = window_control::floating(self.on_top, self.core.dialog_busy);
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
    /// with `appearance`, the file resolvers' extensions (`fileTypes`, from
    /// the `resolvers.fileTypes` event), `language` and `locale`.
    pub(crate) fn window_state(&self) -> Value {
        let facts = &self.core.window_facts;
        json!({"fullscreen":facts["fullscreen"] == true,
            "maximized":reported_maximized(facts),
            "videoArea":{"width":facts["videoArea"]["width"].as_f64().unwrap_or(0.),
                "height":facts["videoArea"]["height"].as_f64().unwrap_or(0.)},
            "pictureInPicture":self.pip_state(),
            "appearance":self.core.theme(),"fileTypes":self.core.file_types,
            "language":self.core.language_preference().unwrap_or_else(|| i18n::SYSTEM.into()),
            "locale":i18n::locale()})
    }
    /// `WindowState.pictureInPicture`: offered here, and the window showing.
    pub(crate) fn pip_state(&self) -> Value {
        let facts = self.pip_facts();
        json!({"supported":facts.window && facts.supported,"active":self.pip.active()})
    }
    /// `window.setVideoArea { width, height }`: resize the window so the video
    /// column gets that size (bars added, fitted to the screen). Answered when
    /// the layout shows it, or after a second. Plan step S2d: the core
    /// checks the size and sends `WindowOp::VideoArea`; the interface fits
    /// the window (`video_area_command`) and reports `UiReport::AreaFitted`
    /// (or its error); the core keeps the reply (`Core::area_replies`) and
    /// answers it from the layout's `UiReport::VideoArea`.
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
        self.ui_command(UiCommand::Window(WindowOp::VideoArea {
            reply: reply.clone(),
            width,
            height,
        }));
        Ok(())
    }
    /// The interface's side of `WindowOp::VideoArea` (plan step S2d): fit
    /// the main window and report the size the video waits for
    /// (`UiReport::AreaFitted`), or why it cannot (`UiReport::Answered`).
    pub(crate) fn video_area_command(&mut self, reply: Reply, width: f64, height: f64) {
        match self.fit_video_area(width, height) {
            Ok((video, result)) => self.ui_report(UiReport::AreaFitted {
                reply,
                video,
                result,
            }),
            Err(error) => self.ui_report(UiReport::Answered {
                reply,
                result: Err(error),
                handoff: None,
                confirm_failed: false,
            }),
        }
    }
    fn fit_video_area(
        &mut self,
        width: f64,
        height: f64,
    ) -> std::result::Result<((f64, f64), Value), RpcError> {
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
        // The adjust column, while docked, takes its width beside the video.
        let adjust = self.window_facts["slots"]["adjustWidth"]
            .as_f64()
            .filter(|width| *width > 0.)
            .map_or(0., |width| width + window_state::SPLITTER_WIDTH);
        let frame = window_control::Frame {
            extra_height: (number("logical-height") - number("video-height")).max(0.),
            extra_width: adjust
                + if docked {
                    self.policy.sidebar_user_width() + window_state::SPLITTER_WIDTH
                } else {
                    0.
                },
            max_content: platform::max_content(&screen),
        };
        let fit = window_control::fit(width, height, frame);
        platform::resize_content(id, fit.content.0, fit.content.1).map_err(|e| unavailable(e.0))?;
        Ok((
            fit.video,
            json!({"width":fit.video.0,"height":fit.video.1,"clamped":fit.clamped}),
        ))
    }
    /// The core's side of `UiReport::VideoArea` (plan step S2d): answer the
    /// pending `window.setVideoArea` requests the laid-out `video` size
    /// shows, or whose deadline passed by `now`.
    pub(crate) fn answer_video_area(&mut self, video: (f64, f64), now: u64) {
        for pending in self.core.due_area_replies(video, now) {
            self.respond(pending.reply, Ok(pending.result));
        }
    }
    /// The window settings in effect reach the interface's copies
    /// (`Core::window_settings`; plan step S2c(7)), in place, right where
    /// it reads them (`apply_on_top`, `window_tick`), as the binding table
    /// and the command registry do (`sync_tables`).
    pub(crate) fn sync_window_settings(&mut self) {
        for command in self.core.window_settings() {
            self.ui_command(command);
        }
    }
    /// The core's info panels changed (`osd.panel`, a plugin stopped): the
    /// interface shows its new copy (`UiSnapshot::InfoPanels`; plan step
    /// S2c(7)), in place.
    pub(crate) fn info_panels_changed(&mut self) {
        let panels = Arc::new(self.core.info_panels.clone());
        self.ui_command(UiCommand::Snapshot(UiSnapshot::InfoPanels(panels)));
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
        if self.core.window_facts["shellGeneration"].as_u64() != Some(self.shell_generation) {
            return Err(RpcError::new(
                "pending",
                "current shell layout is not yet confirmed",
            ));
        }
        // `panels`: the slot's panels in tab order (sidebar tabs); `owner`
        // is the current one's.
        let bottom = self
            .core
            .mounts
            .values()
            .filter(|m| m.spec.slot == composition::Slot::Bottom)
            .map(|m| m.spec.owner)
            .collect::<Vec<_>>();
        // The tab orders from the core's copy (plan step S2c(5)).
        #[cfg(debug_assertions)]
        self.check_panel_tabs();
        let sidebar = &self.core.panel_tabs.sidebar;
        let adjust = &self.core.panel_tabs.adjust;
        let slot = |name: &str| {
            let slot = &self.core.window_facts["slots"][name];
            if slot.is_null() {
                return Value::Null;
            }
            let owners = match name {
                "sidebar" => sidebar,
                "adjust" => adjust,
                _ => &bottom,
            };
            let panels = owners
                .iter()
                .filter_map(|owner| self.core.mounts.get(owner))
                .map(|m| json!({"owner":m.spec.owner,"plugin":m.manifest,"title":m.title}))
                .collect::<Vec<_>>();
            json!({"owner":slot["owner"],"preferred":slot["preferred"],"effective":slot["effective"],
                "reason":slot["reason"],"detached":slot["detached"],"panels":panels})
        };
        Ok(
            json!({"bottom":slot("bottom"),"sidebar":slot("sidebar"),"adjust":slot("adjust"),"backdrop":self.backdrop_state()}),
        )
    }
    /// `panels.backdrop.state`: `{ owner, effective }`, null without a backdrop.
    pub(crate) fn backdrop_state(&self) -> Value {
        self.core
            .mounts
            .values()
            .find(|m| m.spec.slot == composition::Slot::Backdrop)
            .map_or(
                Value::Null,
                |m| json!({"owner":m.spec.owner,"effective":self.no_video}),
            )
    }
}
impl Core {
    /// The core's side of `UiReport::AreaFitted` (plan step S2d): the
    /// fitted `window.setVideoArea` waits for the layout to show `video`,
    /// at most a second from now.
    pub(crate) fn area_fitted(&mut self, reply: Reply, video: (f64, f64), result: Value) {
        let now = self.now_ms();
        area_fitted(&mut self.area_replies, reply, video, result, now);
    }
    /// The pending `window.setVideoArea` requests that are due
    /// (`due_area_replies`).
    pub(crate) fn due_area_replies(&mut self, video: (f64, f64), now: u64) -> Vec<AreaReply> {
        due_area_replies(&mut self.area_replies, video, now)
    }
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
    /// R-U3: a mounted backdrop shows audio files itself: mpv stops picking
    /// cover art as a video track while one is mounted (from the next file
    /// opened). Reads and writes only the core's state; called from the
    /// window tick as before plan step S2c(7) moved it to the core.
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
    /// The values of `window.setAlwaysOnTop` and
    /// `setCursorAutoHide("playing")` in effect that the interface has not
    /// been given yet (plan step S2c(7)): `WindowOp::OnTop` and
    /// `UiSnapshot::CursorAutoHide`, each only when it changed.
    pub(crate) fn window_settings(&mut self) -> Vec<UiCommand> {
        let mut commands = Vec::new();
        let (on_top, playing) = (self.on_top.get(), self.cursor_playing.get());
        if self.window_settings_sent.0 != Some(on_top) {
            self.window_settings_sent.0 = Some(on_top);
            commands.push(UiCommand::Window(WindowOp::OnTop(on_top)));
        }
        if self.window_settings_sent.1 != Some(playing) {
            self.window_settings_sent.1 = Some(playing);
            commands.push(UiCommand::Snapshot(UiSnapshot::CursorAutoHide(playing)));
        }
        commands
    }
    /// The palette (`plugin.appearance`), the core's: kept in the state file.
    pub(crate) fn theme(&self) -> &str {
        self.ui_state
            .state
            .appearance
            .as_ref()
            .map_or(crate::ui::theme::THEMES[0], |a| a.theme.as_str())
    }
    /// The saved interface language, `None` = follow the system.
    pub(crate) fn language_preference(&self) -> Option<String> {
        self.ui_state
            .state
            .appearance
            .as_ref()
            .and_then(|a| a.language.clone())
    }
    /// The interface's part of the state file, given whole at `at` (plan
    /// step S2c(7), `UiReport::Remembered`): written with the core's part
    /// a second later, as the interface's own change was before.
    pub(crate) fn remember(&mut self, state: ui_state::Interface, at: u64) {
        self.ui_state.state.set_interface(state);
        self.ui_state.touch(at);
    }
}

/// The core's side of `UiReport::AreaFitted` (plan step S2d): the fitted
/// `window.setVideoArea` waits in `pending` for the layout to show `video`,
/// at most a second from `now`, the report's receipt.
pub(crate) fn area_fitted(
    pending: &mut Vec<AreaReply>,
    reply: Reply,
    video: (f64, f64),
    result: Value,
    now: u64,
) {
    pending.push(AreaReply {
        reply,
        video,
        result,
        deadline: now + 1000,
    });
}

/// The pending `window.setVideoArea` requests that are due: the laid-out
/// `video` size shows theirs (within a pixel), or their deadline passed by
/// `now`. They leave `pending` in the order they came; each is answered
/// with the result the interface computed when it fitted the window, also
/// on the timeout path (today's reply).
pub(crate) fn due_area_replies(
    pending: &mut Vec<AreaReply>,
    video: (f64, f64),
    now: u64,
) -> Vec<AreaReply> {
    let (done, waiting): (Vec<_>, Vec<_>) = pending.drain(..).partition(|pending| {
        now >= pending.deadline
            || ((video.0 - pending.video.0).abs() <= 1. && (video.1 - pending.video.1).abs() <= 1.)
    });
    *pending = waiting;
    done
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
        // Menu redesign step 3b: the adjust column does not detach.
        assert_eq!(code(json!({"detached":true}), Slot::Adjust), "invalid");
        assert_eq!(
            layout_change(
                &json!({"detached":false,"visible":true}),
                Slot::Adjust,
                &panels
            )
            .unwrap()
            .visible,
            Some(true)
        );
        assert_eq!(policy_slot(Slot::Adjust), Some(window_state::Slot::Adjust));
    }

    /// The policy's tabs as the interface reports them (`Host::policy_tabs`).
    fn tabs_of(policy: &window_state::Policy) -> PanelTabs {
        PanelTabs {
            bottom: policy.bottom(),
            sidebar: policy.sidebar_panels(),
            adjust: policy.panels(window_state::Slot::Adjust),
        }
    }

    /// `Host::layout_apply`'s steps on `policy`: on a refusal, the error and
    /// whether an earlier step took effect.
    fn apply_layout(
        policy: &mut window_state::Policy,
        slot: composition::Slot,
        policy_slot: window_state::Slot,
        change: &LayoutChange,
        first: u64,
    ) -> std::result::Result<(), (window_state::Error, bool)> {
        let owner = slot
            .tabbed()
            .then(|| policy.active(policy_slot))
            .flatten()
            .unwrap_or(first);
        let mut changed = false;
        if let Some(active) = change.active
            && slot.tabbed()
        {
            policy.set_active(active).map_err(|e| (e, changed))?;
            changed = true;
        }
        if let Some(detached) = change.detached {
            policy
                .set_detached(owner, detached)
                .map_err(|e| (e, changed))?;
            changed = true;
        }
        if let Some(visible) = change.visible {
            policy
                .set_preferred(policy_slot, owner, visible, 0)
                .map_err(|e| (e, changed))?;
        }
        Ok(())
    }

    /// Plan step S2c(5): what the core judges from its copy of the tabs is
    /// the policy's own answer, for every panel and change. `holds` is
    /// `set_locks`'s and `set_preferred`'s answer; `judge_layout` refuses
    /// exactly what the policy refuses before any step took effect, and
    /// leaves a later refusal (after a step took effect) to the interface.
    #[test]
    fn the_cores_judgement_from_the_tabs_is_the_policys() {
        use composition::Slot as C;
        let slots = [C::Bottom, C::Sidebar, C::Adjust];
        let layouts: Vec<Vec<(C, u64)>> = vec![
            vec![],
            vec![(C::Bottom, 1)],
            vec![(C::Sidebar, 1), (C::Sidebar, 2)],
            vec![(C::Adjust, 3)],
            vec![
                (C::Bottom, 1),
                (C::Sidebar, 2),
                (C::Adjust, 3),
                (C::Adjust, 4),
            ],
        ];
        let keys: Vec<u64> = (1..=5)
            .flat_map(|owner| slots.map(|slot| panel_key(owner, slot)))
            .collect();
        let flags = [None, Some(false), Some(true)];
        let mut refused = 0;
        let mut deferred = 0;
        for layout in &layouts {
            let mut policy = window_state::Policy::new(0);
            for &(slot, owner) in layout {
                policy
                    .attach(
                        policy_slot(slot).unwrap(),
                        panel_key(owner, slot),
                        true,
                        false,
                    )
                    .unwrap();
            }
            let tabs = tabs_of(&policy);
            for slot in slots {
                let ps = policy_slot(slot).unwrap();
                for &key in &keys {
                    let locks = window_state::Locks::default();
                    assert_eq!(
                        tabs.holds(ps, key),
                        policy.clone().set_locks(ps, key, locks)
                    );
                    assert_eq!(
                        tabs.holds(ps, key),
                        policy.clone().set_preferred(ps, key, true, 0)
                    );
                    assert_eq!(tabs.holds(ps, key), policy.clone().switch(ps, key, true, 0));
                }
                for &first in &keys {
                    for active in std::iter::once(None).chain(keys.iter().copied().map(Some)) {
                        for detached in flags {
                            // `layout_change` refuses these first.
                            if (detached.is_some() && slot == C::Bottom)
                                || (detached == Some(true) && slot == C::Adjust)
                            {
                                continue;
                            }
                            for visible in flags {
                                let change = LayoutChange {
                                    visible,
                                    detached,
                                    active,
                                };
                                let real =
                                    apply_layout(&mut policy.clone(), slot, ps, &change, first);
                                let judged = tabs.judge_layout(slot, ps, &change, first);
                                let expected = match real {
                                    Ok(()) => Ok(()),
                                    Err((_, true)) => {
                                        deferred += 1;
                                        Ok(())
                                    }
                                    Err((error, false)) => {
                                        refused += 1;
                                        Err(error)
                                    }
                                };
                                assert_eq!(
                                    judged, expected,
                                    "{layout:?} {slot:?} first {first} {change:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
        // Both kinds of refusal occur.
        assert!(refused > 0 && deferred > 0, "{refused} {deferred}");
    }
}

/// The `window.setVideoArea` and `window.setPictureInPicture` seam of plan
/// step S2d, on the core's side with the interface played by the test
/// (offscreen runs have no native window: both are refused at once there).
#[cfg(test)]
mod seam_tests {
    use super::*;

    fn reply(id: u64) -> Reply {
        Reply { owner: 7, id }
    }
    fn result(width: f64, height: f64) -> Value {
        json!({"width":width,"height":height,"clamped":false})
    }
    fn ids(done: &[AreaReply]) -> Vec<u64> {
        done.iter().map(|pending| pending.reply.id).collect()
    }

    /// R5 / R13 (case 2): nothing is answered before the interface reported
    /// the fitted size; afterwards the reply leaves at the first layout that
    /// shows it (within a pixel), with the result the interface computed.
    #[test]
    fn an_area_is_answered_only_after_the_ui_fitted_it_and_its_layout_shows_it() {
        let mut pending = Vec::new();
        assert!(due_area_replies(&mut pending, (640., 360.), 10).is_empty());
        area_fitted(
            &mut pending,
            reply(1),
            (640., 360.),
            result(640., 360.),
            100,
        );
        assert!(due_area_replies(&mut pending, (800., 450.), 116).is_empty());
        let done = due_area_replies(&mut pending, (640.5, 359.2), 132);
        assert_eq!(ids(&done), [1]);
        assert_eq!(done[0].result, result(640., 360.));
        assert!(pending.is_empty());
    }

    /// The 1 s timeout path (design §5): a size the layout never shows is
    /// answered at receipt + 1000 ms, not a millisecond earlier, with
    /// today's reply (the size the host computed).
    #[test]
    fn an_area_never_shown_is_answered_after_one_second_with_todays_reply() {
        let mut pending = Vec::new();
        area_fitted(
            &mut pending,
            reply(2),
            (1920., 1080.),
            result(1920., 1080.),
            500,
        );
        assert!(due_area_replies(&mut pending, (1280., 720.), 1499).is_empty());
        let done = due_area_replies(&mut pending, (1280., 720.), 1500);
        assert_eq!(ids(&done), [2]);
        assert_eq!(done[0].result, result(1920., 1080.));
    }

    /// Several pending: the shown one and the timed-out one leave together,
    /// in the order they came; the others wait.
    #[test]
    fn due_areas_leave_in_arrival_order_and_the_rest_wait() {
        let mut pending = Vec::new();
        area_fitted(&mut pending, reply(1), (640., 360.), result(640., 360.), 0);
        area_fitted(
            &mut pending,
            reply(2),
            (800., 450.),
            result(800., 450.),
            600,
        );
        area_fitted(
            &mut pending,
            reply(3),
            (320., 180.),
            result(320., 180.),
            700,
        );
        let done = due_area_replies(&mut pending, (320., 180.), 1000);
        assert_eq!(ids(&done), [1, 3]);
        assert_eq!(ids(&pending), [2]);
    }

    /// The window tick of the source after `needle`.
    fn after<'a>(source: &'a str, needle: &str) -> &'a str {
        &source[source.find(needle).expect(needle)..]
    }

    /// R13's wire order for `window.setVideoArea` (and today's): in the
    /// window tick the layout's size goes to the core (`VideoArea`: the
    /// replies) before the facts (`WindowFacts`: the `window.state`
    /// broadcast); and the core's handling of a UI result applies the
    /// handoff, then replies, then broadcasts.
    #[test]
    fn the_area_reply_goes_before_the_window_state_broadcast() {
        let window = include_str!("window.rs");
        let tick = after(window, "fn window_tick(");
        let area = tick.find("UiReport::VideoArea {").expect("area report");
        let facts = tick
            .find("self.ui_report(UiReport::WindowFacts(")
            .expect("facts report");
        assert!(area < facts, "the area replies before the facts");
        let answered = after(after(window, "fn ui_report("), "UiReport::Answered {");
        let apply = answered.find("apply_facts").expect("handoff applied");
        let respond = answered.find("self.respond(reply, result)").expect("reply");
        let broadcast = answered.find("\"window.state\"").expect("broadcast");
        assert!(apply < respond && respond < broadcast);
    }

    /// `window.setPictureInPicture` (R13 case 3, as built in S2d): its
    /// results reach the core as `Answered` without a handoff, so the core
    /// broadcasts nothing with the reply; `window.state` keeps coming from
    /// the window tick's facts, as today, and reads the live PiP state.
    #[test]
    fn a_picture_in_picture_reply_carries_no_handoff() {
        let video = include_str!("video.rs");
        let answered = after(video, "fn pip_answered(");
        let body = &answered[..answered.find("\n    }").expect("end")];
        assert!(body.contains("handoff: None"));
        assert!(
            after(video, "fn pip_tick(").contains(
                "pip::Effect::Respond(reply, result) => self.pip_answered(reply, result)"
            )
        );
        let state = include_str!("state.rs");
        assert!(after(state, "fn window_state(").contains("\"pictureInPicture\":self.pip_state()"));
    }
}
