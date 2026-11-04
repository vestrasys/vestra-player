//! Finite shell visibility policy (host-internal).
//! No OS calls, clocks, resizing, plugin identity, or layout description protocol.
//! The host supplies monotonic elapsed milliseconds and usable logical pixels.
#![forbid(unsafe_code)]

use serde::Serialize;

const IDLE_MS: u64 = 3000;
/// G-01: the fullscreen sidebar pops out when the pointer comes this close to
/// the right edge (Vestra value).
pub const EDGE_PX: f64 = 4.0;
/// G-01: the popped-out sidebar collapses this long after the pointer left it
/// (Vestra value). FS-BAR-1: the fullscreen bar hides this long after the
/// pointer left its band.
pub const LEAVE_MS: u64 = 500;
/// UI-SHELL-06/08: video at least 318px, a 3px splitter, sidebar at least 317px.
pub const VIDEO_MIN_WIDTH: f64 = 318.0;
pub const SPLITTER_WIDTH: f64 = 3.0;
pub const SIDEBAR_MIN_WIDTH: f64 = 317.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Bottom,
    Sidebar,
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    /// The window's logical content size; the sidebar spans its full height.
    pub width: f64,
    pub height: f64,
}

/// Docked sidebar width for a window `width` wide and a user width.
pub fn sidebar_width(width: f64, user: f64) -> f64 {
    user.min(width - VIDEO_MIN_WIDTH - SPLITTER_WIDTH)
        .max(SIDEBAR_MIN_WIDTH)
}

/// The fullscreen sidebar overlay's width: the user's docked width, leaving
/// at least the minimum video width uncovered.
pub fn overlay_width(width: f64, user: f64) -> f64 {
    user.min(width - VIDEO_MIN_WIDTH).max(0.0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    pub focused: bool,
    pub dragging: bool,
    pub popup_open: bool,
}

impl Locks {
    fn active(self) -> bool {
        self.focused || self.dragging || self.popup_open
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    UserHidden,
    Space,
    Fullscreen,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Visibility {
    pub owner: u64,
    /// Persistent ordinary-window preference; fullscreen overrides never alter it.
    pub preferred: bool,
    pub effective: bool,
    pub reason: Option<Reason>,
    pub overlay: bool,
    /// The sidebar is shown in its own floating window (C14).
    pub detached: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub fullscreen: bool,
    pub bottom: Option<Visibility>,
    pub sidebar: Option<Visibility>,
    /// Actual allocated sidebar width, zero whenever the sidebar is not shown
    /// docked (hidden, floating, or the fullscreen overlay, which takes none).
    pub sidebar_width: f64,
    /// The fullscreen bar's pending hide (the leave delay running); absent
    /// while it is held or already hidden.
    pub next_deadline_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Occupied,
    MissingOwner,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid viewport",
            Self::Occupied => "visibility slot is already attached",
            Self::MissingOwner => {
                "visibility slot is absent or belongs to another owner generation"
            }
        })
    }
}
impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug)]
struct Attachment {
    owner: u64,
    preferred: bool,
    locks: Locks,
}

/// The sidebar: one or more panels as tabs (panel-slots.md step 1). Shown,
/// hidden and detached as a whole; one tab is current.
#[derive(Clone, Debug)]
struct Tabs {
    /// Tab order, with each panel's own interaction locks.
    panels: Vec<(u64, Locks)>,
    active: u64,
    preferred: bool,
    detached: bool,
}

impl Tabs {
    fn has(&self, owner: u64) -> bool {
        self.panels.iter().any(|(o, _)| *o == owner)
    }
    /// Only the current tab's locks hold the popped-out overlay.
    fn active_locks(&self) -> Locks {
        self.panels
            .iter()
            .find(|(o, _)| *o == self.active)
            .map(|(_, locks)| *locks)
            .unwrap_or_default()
    }
}

/// G-01: the attached sidebar in fullscreen is an overlay at the right edge,
/// collapsed or popped out.
#[derive(Clone, Copy, Debug, Default)]
struct Overlay {
    open: bool,
    /// The pointer has been over the popped-out panel since it opened (a
    /// panel popped out by its toggle waits for that before it can leave).
    entered: bool,
    /// When the pointer was first seen over the video, left of the panel,
    /// after being over it.
    left_at: Option<u64>,
}

#[derive(Clone)]
pub struct Policy {
    bottom: Option<Attachment>,
    sidebar: Option<Tabs>,
    fullscreen: bool,
    last_activity_ms: u64,
    menu_open: bool,
    dialog_open: bool,
    sidebar_user_width: f64,
    /// G-04: the pointer is over the panel (host-owned, never cleared by a
    /// plugin's `setInteraction`), by slot: bottom, sidebar.
    hover: [bool; 2],
    overlay: Overlay,
    bar: Bar,
}

/// FS-BAR-1: the fullscreen bottom bar shows only while the pointer is in
/// its band (or a bar lock holds it), and hides `LEAVE_MS` after that ends.
/// Activity (motion elsewhere, keys, menus, dialogs) never reveals it.
#[derive(Clone, Copy, Debug, Default)]
struct Bar {
    /// The pointer is in the bar's band (host-sampled every tick).
    band: bool,
    /// A menu or dialog opened while the bar showed holds it until closed.
    obstructed: bool,
    /// Something held the bar at the last update.
    held: bool,
    /// When the hold ended: the leave delay runs from here.
    left_at: Option<u64>,
    /// The bar showed at the last update (for `set_obstructions`).
    shown: bool,
}

impl Policy {
    pub fn new(now_ms: u64) -> Self {
        Self {
            bottom: None,
            sidebar: None,
            fullscreen: false,
            last_activity_ms: now_ms,
            menu_open: false,
            dialog_open: false,
            sidebar_user_width: SIDEBAR_MIN_WIDTH,
            hover: [false; 2],
            overlay: Overlay::default(),
            bar: Bar::default(),
        }
    }

    /// The width the user dragged to (or restored); the docked width is clamped.
    pub fn set_sidebar_width(&mut self, width: f64) {
        if width.is_finite() {
            self.sidebar_user_width = width.max(SIDEBAR_MIN_WIDTH);
        }
    }

    pub fn sidebar_user_width(&self) -> f64 {
        self.sidebar_user_width
    }

    /// Float or dock the whole sidebar; `owner` is any of its panels.
    pub fn set_detached(&mut self, owner: u64, detached: bool) -> Result<()> {
        self.tabs_mut(owner)?.detached = detached;
        self.overlay = Overlay::default();
        Ok(())
    }

    fn tabs_mut(&mut self, owner: u64) -> Result<&mut Tabs> {
        self.sidebar
            .as_mut()
            .filter(|tabs| tabs.has(owner))
            .ok_or(Error::MissingOwner)
    }

    fn bottom_mut(&mut self, owner: u64) -> Result<&mut Attachment> {
        self.bottom
            .as_mut()
            .filter(|attachment| attachment.owner == owner)
            .ok_or(Error::MissingOwner)
    }

    /// The bottom bar takes one panel. The sidebar takes another tab; the
    /// first one sets the slot's `preferred` and `detached` (the slot's
    /// remembered state) and becomes the current tab.
    pub fn attach(
        &mut self,
        slot: Slot,
        owner: u64,
        preferred: bool,
        detached: bool,
    ) -> Result<()> {
        match slot {
            Slot::Bottom => {
                if self.bottom.is_some() {
                    return Err(Error::Occupied);
                }
                self.bottom = Some(Attachment {
                    owner,
                    preferred,
                    locks: Locks::default(),
                });
            }
            Slot::Sidebar => match &mut self.sidebar {
                Some(tabs) if tabs.has(owner) => return Err(Error::Occupied),
                Some(tabs) => tabs.panels.push((owner, Locks::default())),
                None => {
                    self.sidebar = Some(Tabs {
                        panels: vec![(owner, Locks::default())],
                        active: owner,
                        preferred,
                        detached,
                    });
                }
            },
        }
        Ok(())
    }

    /// Put the sidebar's tabs in this order; tabs it does not list keep
    /// their relative order after the listed ones.
    pub fn order_sidebar(&mut self, order: &[u64]) {
        if let Some(tabs) = &mut self.sidebar {
            tabs.panels.sort_by_key(|(owner, _)| {
                order.iter().position(|o| o == owner).unwrap_or(usize::MAX)
            });
        }
    }

    /// Make `owner`'s panel the current sidebar tab.
    pub fn set_active(&mut self, owner: u64) -> Result<()> {
        self.tabs_mut(owner)?.active = owner;
        Ok(())
    }

    /// A sidebar tab's plugin woke from standby as `to` (sidebar-tabs.md):
    /// the tab keeps its place, locks and current state.
    pub fn rekey(&mut self, from: u64, to: u64) {
        if let Some(tabs) = &mut self.sidebar {
            for (owner, _) in &mut tabs.panels {
                if *owner == from {
                    *owner = to;
                }
            }
            if tabs.active == from {
                tabs.active = to;
            }
        }
    }

    /// The sidebar's panels in tab order (empty without a sidebar).
    pub fn sidebar_panels(&self) -> Vec<u64> {
        self.sidebar
            .iter()
            .flat_map(|tabs| tabs.panels.iter().map(|(owner, _)| *owner))
            .collect()
    }

    /// The sidebar slot's own state: (preferred, detached).
    pub fn sidebar_state(&self) -> Option<(bool, bool)> {
        self.sidebar
            .as_ref()
            .map(|tabs| (tabs.preferred, tabs.detached))
    }

    /// The current sidebar tab.
    pub fn sidebar_active(&self) -> Option<u64> {
        self.sidebar.as_ref().map(|tabs| tabs.active)
    }

    /// Removing the current tab makes the next one current (the previous
    /// one when it was the last); the slot keeps its state while a tab is
    /// left.
    pub fn remove_owner(&mut self, owner: u64) {
        if self
            .bottom
            .is_some_and(|attachment| attachment.owner == owner)
        {
            self.bottom = None;
        }
        if let Some(tabs) = &mut self.sidebar
            && let Some(index) = tabs.panels.iter().position(|(o, _)| *o == owner)
        {
            tabs.panels.remove(index);
            match tabs.panels.get(index).or(tabs.panels.last()) {
                None => {
                    self.sidebar = None;
                    self.overlay = Overlay::default();
                }
                Some((next, _)) if tabs.active == owner => tabs.active = *next,
                Some(_) => {}
            }
        }
    }

    /// G-01: in fullscreen, turning the attached sidebar on pops it out at
    /// once (until the pointer has been over it and left, or 3 seconds pass
    /// without activity); turning it off collapses it at once.
    pub fn set_preferred(
        &mut self,
        slot: Slot,
        owner: u64,
        preferred: bool,
        now_ms: u64,
    ) -> Result<()> {
        let current = match slot {
            Slot::Bottom => &mut self.bottom_mut(owner)?.preferred,
            Slot::Sidebar => &mut self.tabs_mut(owner)?.preferred,
        };
        let shown = !*current && preferred;
        *current = preferred;
        if slot == Slot::Sidebar {
            if !preferred {
                self.overlay = Overlay::default();
            } else if shown && self.overlay_ready() {
                self.overlay = Overlay {
                    open: true,
                    entered: false,
                    left_at: None,
                };
                self.activity(now_ms);
            }
        }
        Ok(())
    }

    /// Setters do not invent timestamps. A released gesture counts as activity
    /// when the host calls activity(release_now) first; the fullscreen bar's
    /// leave delay starts at the next `tick` or `bottom_band`.
    /// A sidebar panel's locks count only while its tab is current.
    pub fn set_locks(&mut self, slot: Slot, owner: u64, locks: Locks) -> Result<()> {
        match slot {
            Slot::Bottom => self.bottom_mut(owner)?.locks = locks,
            Slot::Sidebar => {
                let tabs = self.tabs_mut(owner)?;
                if let Some(entry) = tabs.panels.iter_mut().find(|(o, _)| *o == owner) {
                    entry.1 = locks;
                }
            }
        }
        Ok(())
    }

    /// Repeated observation of the same mode is a no-op. Entering counts as
    /// activity (the pointer shows) but does not reveal the bar (FS-BAR-1).
    pub fn set_fullscreen(&mut self, fullscreen: bool, now_ms: u64) {
        if self.fullscreen == fullscreen {
            return;
        }
        self.fullscreen = fullscreen;
        // Entering fullscreen starts collapsed; leaving forgets the overlay
        // and the bar's band state.
        self.overlay = Overlay::default();
        self.bar = Bar::default();
        if fullscreen {
            self.activity(now_ms);
        }
    }

    /// G-04: whether the pointer is over a slot's panel (the bottom bar, or
    /// the popped-out sidebar). Hovering locks the panel like a plugin's
    /// interaction lock. The host samples it every tick.
    pub fn set_hover(&mut self, slot: Slot, hovered: bool) {
        self.hover[slot as usize] = hovered;
        if slot == Slot::Sidebar && hovered && self.overlay.open {
            self.overlay.entered = true;
            self.overlay.left_at = None;
        }
    }

    /// G-01: the pointer is over the video at `x` (window coordinates) in a
    /// viewport `width` wide. At the right edge the fullscreen sidebar pops
    /// out; left of the popped-out panel the leave delay starts.
    pub fn pointer(&mut self, x: f64, width: f64, now_ms: u64) {
        if !x.is_finite() || !width.is_finite() || !self.overlay_ready() {
            return;
        }
        if x >= width - EDGE_PX {
            if !self.overlay.open {
                self.overlay = Overlay {
                    open: true,
                    entered: true,
                    left_at: None,
                };
                self.activity(now_ms);
            }
        } else if self.overlay.open
            && self.overlay.entered
            && self.overlay.left_at.is_none()
            && !self.hover[Slot::Sidebar as usize]
            && x < width - overlay_width(width, self.sidebar_user_width)
        {
            self.overlay.left_at = Some(now_ms);
        }
    }

    /// FS-BAR-1: where the pointer is for the fullscreen bar's band. The band
    /// is the bar's own rectangle: from `height - band_height` (the bar's
    /// laid-out height, also while hidden) to the bottom edge, left of a
    /// popped-out sidebar. `over_bar`: the pointer is over the shown bar;
    /// `over_video`: the pointer over the video at (x, y), window logical
    /// px. Outside fullscreen there is no band. The host samples it every
    /// tick, before `tick`.
    pub fn bottom_band(
        &mut self,
        over_bar: bool,
        over_video: Option<(f64, f64)>,
        viewport: Viewport,
        band_height: f64,
        now_ms: u64,
    ) {
        let right = viewport.width
            - if self.overlay.open && self.overlay_ready() {
                overlay_width(viewport.width, self.sidebar_user_width)
            } else {
                0.0
            };
        let in_video_band = over_video.is_some_and(|(x, y)| {
            x.is_finite()
                && y.is_finite()
                && band_height.is_finite()
                && band_height > 0.0
                && y >= viewport.height - band_height
                && x < right
        });
        self.bar.band = self.fullscreen && self.bottom.is_some() && (over_bar || in_video_band);
        self.update_bar(now_ms);
    }

    /// Something holds the fullscreen bar now: the pointer in its band or
    /// over it, a bar lock, or a menu or dialog opened while it showed.
    fn bar_holding(&self) -> bool {
        self.bar.band || self.bar.obstructed || self.bottom_locked()
    }

    /// The fullscreen bar shows: held, or the leave delay still running.
    /// A hold that ended since the last update still counts until the next
    /// update starts the delay.
    fn bar_shown(&self, now_ms: u64) -> bool {
        self.bar_holding()
            || self.bar.held
            || self
                .bar
                .left_at
                .is_some_and(|at| now_ms.saturating_sub(at) < LEAVE_MS)
    }

    fn update_bar(&mut self, now_ms: u64) {
        let holding = self.bar_holding();
        if holding {
            self.bar.left_at = None;
        } else if self.bar.held {
            self.bar.left_at = Some(now_ms);
        }
        self.bar.held = holding;
        self.bar.shown = self.fullscreen && self.bar_shown(now_ms);
    }

    /// Apply the timed transitions (the popped-out sidebar collapsing, the
    /// fullscreen bar's leave delay). The host calls this every tick before
    /// `evaluate`.
    pub fn tick(&mut self, now_ms: u64) {
        self.update_bar(now_ms);
        if !self.overlay.open {
            return;
        }
        if !self.overlay_ready() {
            self.overlay = Overlay::default();
            return;
        }
        if self.sidebar_locked() {
            return;
        }
        let left = self
            .overlay
            .left_at
            .is_some_and(|at| now_ms.saturating_sub(at) >= LEAVE_MS);
        let idle = now_ms.saturating_sub(self.last_activity_ms) >= IDLE_MS;
        if left || idle {
            self.overlay = Overlay::default();
        }
    }

    /// The attached sidebar can pop out: fullscreen, shown by preference,
    /// not floating.
    fn overlay_ready(&self) -> bool {
        self.fullscreen
            && self
                .sidebar
                .as_ref()
                .is_some_and(|tabs| tabs.preferred && !tabs.detached)
    }

    /// The popped-out sidebar stays: pointer over it, the panel's own
    /// interaction locks, or a menu or dialog open.
    fn sidebar_locked(&self) -> bool {
        self.menu_open
            || self.dialog_open
            || self.hover[Slot::Sidebar as usize]
            || self
                .sidebar
                .as_ref()
                .is_some_and(|tabs| tabs.active_locks().active())
    }

    fn bottom_locked(&self) -> bool {
        self.hover[Slot::Bottom as usize]
            || self
                .bottom
                .is_some_and(|attachment| attachment.locks.active())
    }

    /// Caller uses a monotonic clock. A backward activity sample cannot rewind
    /// the remembered activity time or underflow idle arithmetic.
    pub fn activity(&mut self, now_ms: u64) {
        self.last_activity_ms = self.last_activity_ms.max(now_ms);
    }

    /// FS-BAR-1: a menu or dialog opening while the fullscreen bar shows
    /// holds it until both are closed; one opening while it is hidden does
    /// not reveal it.
    pub fn set_obstructions(&mut self, menu_open: bool, dialog_open: bool) {
        let opening = (menu_open && !self.menu_open) || (dialog_open && !self.dialog_open);
        if !menu_open && !dialog_open {
            self.bar.obstructed = false;
        } else if opening && self.bar.shown {
            self.bar.obstructed = true;
        }
        self.menu_open = menu_open;
        self.dialog_open = dialog_open;
    }

    /// No activity for 3 seconds, no menu or dialog, the pointer neither over
    /// the bar nor in its fullscreen band, and no bar lock: the pointer may
    /// hide (UI-SHELL-09, `setCursorAutoHide`). The fullscreen bar does not
    /// follow this (FS-BAR-1).
    pub fn idle(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_activity_ms) >= IDLE_MS
            && !self.menu_open
            && !self.dialog_open
            && !self.bar.band
            && !self.bottom_locked()
    }

    pub fn evaluate(&self, viewport: Viewport, now_ms: u64) -> Result<Snapshot> {
        if !viewport.width.is_finite()
            || viewport.width < 0.0
            || !viewport.height.is_finite()
            || viewport.height < 0.0
        {
            return Err(Error::Invalid);
        }
        let shown = self.bar_shown(now_ms);
        let bottom = self.bottom.map(|attachment| {
            let reason = if !attachment.preferred {
                Some(Reason::UserHidden)
            } else if self.fullscreen && !shown {
                // FS-BAR-1: fullscreen and the pointer not in the bar's band.
                Some(Reason::Idle)
            } else {
                None
            };
            Visibility {
                owner: attachment.owner,
                preferred: attachment.preferred,
                effective: reason.is_none(),
                reason,
                overlay: self.fullscreen,
                detached: false,
            }
        });
        // Compress the video to its minimum before hiding the docked sidebar.
        let has_space = viewport.width >= VIDEO_MIN_WIDTH + SPLITTER_WIDTH + SIDEBAR_MIN_WIDTH;
        let sidebar = self.sidebar.as_ref().map(|tabs| {
            let reason = if !tabs.preferred {
                Some(Reason::UserHidden)
            } else if tabs.detached {
                // A floating window follows only the preference.
                None
            } else if !has_space {
                Some(Reason::Space)
            } else if self.fullscreen && !self.overlay.open {
                // G-01: the overlay is collapsed (the right edge pops it out).
                Some(Reason::Fullscreen)
            } else {
                None
            };
            Visibility {
                owner: tabs.active,
                preferred: tabs.preferred,
                effective: reason.is_none(),
                reason,
                overlay: self.fullscreen && !tabs.detached,
                detached: tabs.detached,
            }
        });
        let next_deadline_ms = if self.fullscreen
            && !self.bar_holding()
            && !self.bar.held
            && self.bottom.is_some_and(|attachment| attachment.preferred)
        {
            self.bar
                .left_at
                .and_then(|at| at.checked_add(LEAVE_MS))
                .filter(|deadline| *deadline > now_ms)
        } else {
            None
        };
        Ok(Snapshot {
            fullscreen: self.fullscreen,
            bottom,
            sidebar,
            sidebar_width: if sidebar.is_some_and(|v| v.effective && !v.detached && !v.overlay) {
                sidebar_width(viewport.width, self.sidebar_user_width)
            } else {
                0.0
            },
            next_deadline_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn viewport(width: f64, height: f64) -> Viewport {
        Viewport { width, height }
    }
    fn roomy() -> Viewport {
        viewport(1100.0, 600.0)
    }
    fn attached() -> Policy {
        let mut policy = Policy::new(0);
        policy.attach(Slot::Bottom, 1, true, false).unwrap();
        policy.attach(Slot::Sidebar, 2, true, false).unwrap();
        policy
    }

    #[test]
    fn narrow_windows_compress_the_video_before_hiding_the_sidebar() {
        let mut policy = attached();
        let narrow = policy.evaluate(viewport(637.0, 200.0), 0).unwrap();
        assert_eq!(narrow.sidebar.unwrap().reason, Some(Reason::Space));
        assert!(narrow.sidebar.unwrap().preferred);
        assert_eq!(narrow.sidebar_width, 0.0);
        let minimum = policy.evaluate(viewport(638.0, 182.0), 0).unwrap();
        assert!(minimum.sidebar.unwrap().effective);
        assert_eq!(minimum.sidebar_width, 317.0);
        policy.set_sidebar_width(500.0);
        assert_eq!(
            policy
                .evaluate(viewport(700.0, 400.0), 0)
                .unwrap()
                .sidebar_width,
            379.0
        );
        assert_eq!(policy.evaluate(roomy(), 0).unwrap().sidebar_width, 500.0);
        policy.set_detached(2, true).unwrap();
        let floating = policy.evaluate(viewport(320.0, 200.0), 0).unwrap();
        assert!(floating.sidebar.unwrap().effective && floating.sidebar.unwrap().detached);
        assert_eq!(
            floating.sidebar_width, 0.0,
            "a floating panel takes no shell width"
        );
        policy.set_preferred(Slot::Sidebar, 2, false, 0).unwrap();
        assert_eq!(
            policy.evaluate(roomy(), 0).unwrap().sidebar.unwrap().reason,
            Some(Reason::UserHidden)
        );
        assert_eq!(policy.set_detached(9, true), Err(Error::MissingOwner));
    }

    #[test]
    fn fullscreen_hides_the_docked_sidebar_but_keeps_preference_and_floating_window() {
        let mut policy = attached();
        policy.set_fullscreen(true, 100);
        let docked = policy.evaluate(roomy(), 100).unwrap().sidebar.unwrap();
        assert_eq!(docked.reason, Some(Reason::Fullscreen));
        assert!(docked.preferred);
        policy.set_detached(2, true).unwrap();
        assert!(
            policy
                .evaluate(roomy(), 100)
                .unwrap()
                .sidebar
                .unwrap()
                .effective
        );
        policy.set_detached(2, false).unwrap();
        policy.set_fullscreen(false, 200);
        assert!(
            policy
                .evaluate(roomy(), 200)
                .unwrap()
                .sidebar
                .unwrap()
                .effective
        );
        policy.set_preferred(Slot::Sidebar, 2, false, 0).unwrap();
        policy.set_fullscreen(true, 300);
        assert_eq!(
            policy
                .evaluate(roomy(), 300)
                .unwrap()
                .sidebar
                .unwrap()
                .reason,
            Some(Reason::UserHidden)
        );
    }

    #[test]
    fn owner_cleanup_cannot_leave_locks_or_remove_another_generation() {
        let mut policy = attached();
        assert_eq!(
            policy.attach(Slot::Bottom, 3, true, false),
            Err(Error::Occupied)
        );
        assert_eq!(
            policy.set_locks(Slot::Bottom, 3, Locks::default()),
            Err(Error::MissingOwner)
        );
        policy
            .set_locks(
                Slot::Bottom,
                1,
                Locks {
                    dragging: true,
                    ..Locks::default()
                },
            )
            .unwrap();
        policy.set_fullscreen(true, 0);
        policy.remove_owner(1);
        assert!(
            policy
                .evaluate(roomy(), 9000)
                .unwrap()
                .sidebar
                .unwrap()
                .preferred
        );
        policy.attach(Slot::Bottom, 3, true, false).unwrap();
        assert_eq!(
            policy
                .evaluate(roomy(), 9000)
                .unwrap()
                .bottom
                .unwrap()
                .reason,
            Some(Reason::Idle)
        );
        policy.remove_owner(2);
        policy.attach(Slot::Sidebar, 4, true, false).unwrap();
        assert_eq!(
            policy
                .evaluate(roomy(), 9000)
                .unwrap()
                .sidebar
                .unwrap()
                .reason,
            Some(Reason::Fullscreen)
        );
        policy.remove_owner(1); // Late old cleanup cannot remove the replacement.
        assert_eq!(
            policy
                .evaluate(roomy(), 9000)
                .unwrap()
                .bottom
                .unwrap()
                .owner,
            3
        );
    }

    // FS-BAR-1: the fullscreen bar shows only while the pointer is in its
    // band. The screen is 1920 x 1080 and the bar 80 high: the band starts
    // at y = 1000.
    fn bar(policy: &Policy, now: u64) -> Visibility {
        policy.evaluate(screen(), now).unwrap().bottom.unwrap()
    }
    fn at(policy: &mut Policy, x: f64, y: f64, now: u64) {
        policy.bottom_band(false, Some((x, y)), screen(), 80.0, now);
        policy.tick(now);
    }
    fn away(policy: &mut Policy, now: u64) {
        at(policy, 960.0, 500.0, now);
    }
    fn dragging() -> Locks {
        Locks {
            dragging: true,
            ..Locks::default()
        }
    }

    #[test]
    fn fullscreen_bar_ignores_activity_keys_and_entering_fullscreen() {
        let mut policy = attached();
        policy.set_fullscreen(true, 100);
        let entered = policy.evaluate(screen(), 100).unwrap();
        let bottom = entered.bottom.unwrap();
        assert!(
            !bottom.effective,
            "entering fullscreen does not reveal the bar"
        );
        assert_eq!(bottom.reason, Some(Reason::Idle));
        assert!(bottom.preferred && bottom.overlay);
        assert_eq!(entered.next_deadline_ms, None);
        // Pointer motion outside the band, keys and bindings are activity only.
        for now in [200, 1000, 5000] {
            policy.activity(now);
            away(&mut policy, now);
            assert!(!bar(&policy, now).effective);
        }
        policy.activity(6000); // A key seek with the pointer resting mid-screen.
        policy.tick(6000);
        assert!(!bar(&policy, 6000).effective);
        // At the bottom edge but not over the video (outside the window).
        policy.bottom_band(false, None, screen(), 80.0, 7000);
        assert!(!bar(&policy, 7000).effective);
        // The user's preference still wins over the band.
        policy.set_preferred(Slot::Bottom, 1, false, 0).unwrap();
        at(&mut policy, 960.0, 1050.0, 8000);
        policy.set_locks(Slot::Bottom, 1, dragging()).unwrap();
        let hidden = policy.evaluate(screen(), 8000).unwrap();
        assert_eq!(hidden.bottom.unwrap().reason, Some(Reason::UserHidden));
        assert_eq!(hidden.next_deadline_ms, None);
        // Windowed mode is unchanged: docked, never idle.
        policy.set_locks(Slot::Bottom, 1, Locks::default()).unwrap();
        policy.set_preferred(Slot::Bottom, 1, true, 0).unwrap();
        policy.set_fullscreen(false, 9000);
        away(&mut policy, 9000);
        let windowed = bar(&policy, 60_000);
        assert!(windowed.effective && !windowed.overlay);
    }

    #[test]
    fn the_band_reveals_the_bar_at_once_and_it_hides_500_ms_after_leaving() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        at(&mut policy, 960.0, 1000.0, 1000);
        assert!(
            bar(&policy, 1000).effective,
            "entering the band shows it at once"
        );
        assert_eq!(
            policy.evaluate(screen(), 1000).unwrap().next_deadline_ms,
            None
        );
        at(&mut policy, 960.0, 1079.0, 1500);
        assert!(bar(&policy, 1500).effective);
        at(&mut policy, 960.0, 999.0, 2000); // Just above the band.
        assert!(bar(&policy, 2000).effective);
        assert!(bar(&policy, 2499).effective);
        assert_eq!(
            policy.evaluate(screen(), 2499).unwrap().next_deadline_ms,
            Some(2500)
        );
        policy.tick(2500);
        let hidden = policy.evaluate(screen(), 2500).unwrap();
        assert_eq!(hidden.bottom.unwrap().reason, Some(Reason::Idle));
        assert_eq!(hidden.next_deadline_ms, None);
        // Back in within the delay cancels it.
        at(&mut policy, 100.0, 1040.0, 3000);
        away(&mut policy, 3100);
        at(&mut policy, 100.0, 1040.0, 3400);
        policy.tick(3700);
        assert!(bar(&policy, 3700).effective);
        away(&mut policy, 4000);
        policy.tick(4499);
        assert!(bar(&policy, 4499).effective);
        policy.tick(4500);
        assert!(!bar(&policy, 4500).effective);
        // Over the shown bar counts as in the band.
        policy.bottom_band(true, None, screen(), 80.0, 5000);
        assert!(bar(&policy, 5000).effective);
        // The band is the bar's laid-out height in logical px, on any screen.
        let small = viewport(1280.0, 720.0);
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        policy.bottom_band(false, Some((10.0, 655.0)), small, 64.0, 100);
        assert!(
            !policy
                .evaluate(small, 100)
                .unwrap()
                .bottom
                .unwrap()
                .effective
        );
        policy.bottom_band(false, Some((10.0, 656.0)), small, 64.0, 200);
        assert!(
            policy
                .evaluate(small, 200)
                .unwrap()
                .bottom
                .unwrap()
                .effective
        );
        // No laid-out bar, no band.
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        policy.bottom_band(false, Some((10.0, 1079.0)), screen(), 0.0, 100);
        assert!(!bar(&policy, 100).effective);
    }

    #[test]
    fn a_bar_lock_holds_it_after_leaving_and_release_starts_the_delay() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        at(&mut policy, 960.0, 1040.0, 1000);
        policy.set_locks(Slot::Bottom, 1, dragging()).unwrap();
        away(&mut policy, 1100); // Dragging the seek slider up the screen.
        policy.tick(5000);
        let held = policy.evaluate(screen(), 5000).unwrap();
        assert!(held.bottom.unwrap().effective);
        assert_eq!(held.next_deadline_ms, None);
        policy.activity(6000);
        policy.set_locks(Slot::Bottom, 1, Locks::default()).unwrap();
        assert!(
            bar(&policy, 6000).effective,
            "the release before the next tick still shows"
        );
        policy.tick(6000);
        assert!(bar(&policy, 6499).effective);
        assert_eq!(
            policy.evaluate(screen(), 6000).unwrap().next_deadline_ms,
            Some(6500)
        );
        policy.tick(6500);
        assert!(!bar(&policy, 6500).effective);
        // Every lock kind holds; the pointer over the bar too, and a
        // plugin's lock write cannot clear that host-owned hover.
        for locks in [
            Locks {
                focused: true,
                ..Locks::default()
            },
            dragging(),
            Locks {
                popup_open: true,
                ..Locks::default()
            },
        ] {
            policy.set_locks(Slot::Bottom, 1, locks).unwrap();
            policy.tick(9000);
            assert!(bar(&policy, 9000).effective);
        }
        policy.set_locks(Slot::Bottom, 1, Locks::default()).unwrap();
        policy.tick(20_000);
        policy.set_hover(Slot::Bottom, true);
        policy.set_locks(Slot::Bottom, 1, Locks::default()).unwrap();
        policy.tick(30_000);
        assert!(bar(&policy, 30_000).effective);
        policy.set_hover(Slot::Bottom, false);
        policy.tick(30_100);
        assert!(bar(&policy, 30_599).effective);
        assert!(!bar(&policy, 30_600).effective);
    }

    #[test]
    fn a_menu_or_dialog_holds_a_shown_bar_but_never_reveals_a_hidden_one() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        for (menu, dialog) in [(true, false), (false, true)] {
            policy.set_obstructions(menu, dialog);
            policy.tick(1000);
            assert!(!bar(&policy, 1000).effective);
            policy.set_obstructions(false, false);
        }
        // Opened while shown (the bar's own menu, its Open dialog): held.
        at(&mut policy, 960.0, 1040.0, 2000);
        policy.set_obstructions(true, false);
        away(&mut policy, 2100);
        policy.set_obstructions(true, false); // Sampled again every tick.
        policy.tick(5000);
        assert!(bar(&policy, 5000).effective);
        policy.set_obstructions(true, true);
        policy.set_obstructions(false, true);
        policy.tick(5100);
        assert!(bar(&policy, 5100).effective, "the dialog still holds it");
        policy.set_obstructions(false, false);
        policy.tick(6000);
        assert!(bar(&policy, 6499).effective);
        assert!(!bar(&policy, 6500).effective);
        // Opened during the leave delay: the bar still shows, so it holds.
        at(&mut policy, 960.0, 1040.0, 7000);
        away(&mut policy, 7100);
        policy.set_obstructions(false, true);
        policy.tick(9000);
        assert!(bar(&policy, 9000).effective);
    }

    #[test]
    fn the_band_ends_at_a_popped_out_sidebar() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        policy.pointer(1919.0, 1920.0, 100); // Pop the sidebar out (317 wide).
        assert!(sidebar(&policy, 100).effective);
        at(&mut policy, 1700.0, 1050.0, 200);
        assert!(
            !bar(&policy, 200).effective,
            "under the sidebar is not the band"
        );
        at(&mut policy, 1602.0, 1050.0, 300);
        assert!(bar(&policy, 300).effective);
        away(&mut policy, 400);
        policy.tick(1000);
        // A floating sidebar takes nothing from the band.
        policy.set_detached(2, true).unwrap();
        at(&mut policy, 1900.0, 1050.0, 1100);
        assert!(bar(&policy, 1100).effective);
    }

    #[test]
    fn the_cursor_hides_after_three_idle_seconds_but_never_in_the_band() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        assert!(!policy.idle(2999));
        assert!(policy.idle(3000));
        assert!(!bar(&policy, 3000).effective);
        // The pointer moves into the band and rests there.
        policy.activity(4000);
        at(&mut policy, 960.0, 1040.0, 4000);
        assert!(!policy.idle(60_000));
        // A key while the bar is hidden: the cursor counts it, the bar not.
        policy.activity(60_000);
        away(&mut policy, 60_000);
        policy.tick(61_000);
        assert!(!bar(&policy, 61_000).effective);
        assert!(!policy.idle(62_999));
        assert!(policy.idle(63_000));
        policy.activity(64_000);
        assert!(!policy.idle(64_000));
        assert!(!bar(&policy, 64_000).effective);
        // A menu or dialog keeps the cursor.
        policy.set_obstructions(true, false);
        assert!(!policy.idle(90_000));
    }

    fn screen() -> Viewport {
        viewport(1920.0, 1080.0)
    }
    fn sidebar(policy: &Policy, now: u64) -> Visibility {
        policy.evaluate(screen(), now).unwrap().sidebar.unwrap()
    }

    #[test]
    fn fullscreen_sidebar_pops_out_at_the_right_edge_and_collapses_after_the_pointer_leaves() {
        let mut policy = attached();
        policy.set_sidebar_width(400.0);
        policy.set_fullscreen(true, 0);
        let collapsed = sidebar(&policy, 0);
        assert_eq!(collapsed.reason, Some(Reason::Fullscreen));
        assert!(collapsed.overlay && collapsed.preferred && !collapsed.effective);
        policy.pointer(1900.0, 1920.0, 100);
        assert!(!sidebar(&policy, 100).effective, "4 px from the edge only");
        policy.pointer(1916.0, 1920.0, 200);
        let open = policy.evaluate(screen(), 200).unwrap();
        assert!(open.sidebar.unwrap().effective && open.sidebar.unwrap().overlay);
        assert_eq!(open.sidebar.unwrap().reason, None);
        assert_eq!(open.sidebar_width, 0.0, "the overlay takes no layout width");
        // Over the panel: no idle collapse.
        policy.set_hover(Slot::Sidebar, true);
        policy.tick(9000);
        assert!(sidebar(&policy, 9000).effective);
        // Back over the video, left of the panel (1920 - 400 = 1520).
        policy.set_hover(Slot::Sidebar, false);
        policy.pointer(1600.0, 1920.0, 9000);
        policy.activity(9000);
        policy.tick(9600);
        assert!(
            sidebar(&policy, 9600).effective,
            "still over the panel's column"
        );
        policy.pointer(1500.0, 1920.0, 9600);
        policy.activity(9600);
        policy.tick(10_099);
        assert!(sidebar(&policy, 10_099).effective);
        policy.tick(10_100);
        assert_eq!(sidebar(&policy, 10_100).reason, Some(Reason::Fullscreen));
        // Returning before the delay ends keeps it.
        policy.pointer(1919.0, 1920.0, 11_000);
        policy.pointer(1000.0, 1920.0, 11_100);
        policy.set_hover(Slot::Sidebar, true);
        policy.tick(11_700);
        assert!(sidebar(&policy, 11_700).effective);
        // Leaving fullscreen forgets the overlay; entering starts collapsed.
        policy.set_fullscreen(false, 12_000);
        let docked = sidebar(&policy, 12_000);
        assert!(docked.effective && !docked.overlay);
        policy.set_hover(Slot::Sidebar, false);
        policy.set_fullscreen(true, 13_000);
        assert_eq!(sidebar(&policy, 13_000).reason, Some(Reason::Fullscreen));
    }

    #[test]
    fn fullscreen_sidebar_idles_out_unless_locked_and_never_pops_when_hidden_or_floating() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        policy.pointer(1919.0, 1920.0, 1000);
        policy.tick(3999);
        assert!(sidebar(&policy, 3999).effective);
        policy.tick(4000);
        assert!(!sidebar(&policy, 4000).effective, "3 s without activity");
        // A panel lock (search box focused) holds it after the pointer left.
        policy.pointer(1919.0, 1920.0, 5000);
        policy
            .set_locks(
                Slot::Sidebar,
                2,
                Locks {
                    focused: true,
                    ..Locks::default()
                },
            )
            .unwrap();
        policy.pointer(100.0, 1920.0, 5100);
        policy.tick(20_000);
        assert!(sidebar(&policy, 20_000).effective);
        policy
            .set_locks(Slot::Sidebar, 2, Locks::default())
            .unwrap();
        policy.tick(20_000);
        assert!(!sidebar(&policy, 20_000).effective);
        // Hidden by preference: the edge does nothing.
        policy
            .set_preferred(Slot::Sidebar, 2, false, 21_000)
            .unwrap();
        policy.pointer(1919.0, 1920.0, 21_000);
        assert_eq!(sidebar(&policy, 21_000).reason, Some(Reason::UserHidden));
        // Floating: the edge does nothing, the window follows the preference.
        policy
            .set_preferred(Slot::Sidebar, 2, true, 22_000)
            .unwrap();
        policy.set_detached(2, true).unwrap();
        policy.pointer(1919.0, 1920.0, 22_000);
        let floating = sidebar(&policy, 22_000);
        assert!(floating.effective && floating.detached && !floating.overlay);
        policy.tick(40_000);
        assert!(sidebar(&policy, 40_000).effective);
    }

    #[test]
    fn toggling_the_sidebar_in_fullscreen_pops_it_out_or_collapses_it_at_once() {
        let mut policy = attached();
        policy.set_preferred(Slot::Sidebar, 2, false, 0).unwrap();
        policy.set_fullscreen(true, 0);
        policy.set_preferred(Slot::Sidebar, 2, true, 1000).unwrap();
        assert!(sidebar(&policy, 1000).effective, "F6 pops it out at once");
        // The pointer was elsewhere and never over it: no leave delay.
        policy.pointer(200.0, 1920.0, 1100);
        policy.tick(3999);
        assert!(sidebar(&policy, 3999).effective);
        policy.tick(4000);
        assert!(!sidebar(&policy, 4000).effective, "3 s without activity");
        policy.set_preferred(Slot::Sidebar, 2, false, 5000).unwrap();
        policy.set_preferred(Slot::Sidebar, 2, true, 5000).unwrap();
        // Entered and left: the leave delay applies.
        policy.set_hover(Slot::Sidebar, true);
        policy.set_hover(Slot::Sidebar, false);
        policy.pointer(200.0, 1920.0, 5200);
        policy.activity(5200);
        policy.tick(5700);
        assert!(!sidebar(&policy, 5700).effective);
        // F6 again while popped out: collapses at once.
        policy.set_preferred(Slot::Sidebar, 2, true, 6000).unwrap();
        policy.pointer(1919.0, 1920.0, 6000);
        assert!(sidebar(&policy, 6000).effective);
        policy.set_preferred(Slot::Sidebar, 2, false, 6100).unwrap();
        let hidden = sidebar(&policy, 6100);
        assert_eq!(hidden.reason, Some(Reason::UserHidden));
        assert!(!hidden.preferred);
    }

    #[test]
    fn invalid_geometry_and_extreme_timestamps_do_not_produce_fake_layout() {
        let mut policy = attached();
        for view in [
            viewport(f64::NAN, 500.0),
            viewport(1000.0, f64::INFINITY),
            viewport(-1.0, 500.0),
        ] {
            assert_eq!(policy.evaluate(view, 0), Err(Error::Invalid));
        }
        policy.set_fullscreen(true, u64::MAX - 1);
        policy.activity(1); // A delayed sample cannot rewind last activity.
        assert!(!policy.idle(u64::MAX));
        policy.bottom_band(false, Some((10.0, 590.0)), roomy(), 80.0, u64::MAX - 1);
        policy.bottom_band(false, Some((10.0, 10.0)), roomy(), 80.0, u64::MAX - 1);
        let snapshot = policy.evaluate(roomy(), u64::MAX).unwrap();
        assert!(snapshot.bottom.unwrap().effective);
        assert_eq!(snapshot.next_deadline_ms, None); // Unrepresentable future time.
        policy.bottom_band(
            false,
            Some((f64::NAN, 590.0)),
            roomy(),
            f64::INFINITY,
            u64::MAX,
        );
        assert!(!policy.bar.band);
        policy.set_fullscreen(false, u64::MAX);
        assert!(!policy.evaluate(roomy(), u64::MAX).unwrap().fullscreen);
    }

    #[test]
    fn a_tab_whose_plugin_woke_under_a_new_owner_keeps_its_place_and_state() {
        let mut policy = attached();
        policy.attach(Slot::Sidebar, 3, true, false).unwrap();
        policy.attach(Slot::Sidebar, 4, true, false).unwrap();
        policy.set_active(3).unwrap();
        policy.rekey(3, 30);
        assert_eq!(policy.sidebar_panels(), [2, 30, 4]);
        assert_eq!(policy.sidebar_active(), Some(30));
        // A background tab moves alike.
        policy.rekey(4, 40);
        assert_eq!(policy.sidebar_panels(), [2, 30, 40]);
        assert_eq!(policy.sidebar_active(), Some(30));
    }

    #[test]
    fn sidebar_tabs_share_one_state_and_the_next_tab_takes_over_a_removed_current_one() {
        let mut policy = attached();
        // Later panels join as tabs; the slot keeps the first one's state.
        policy.set_preferred(Slot::Sidebar, 2, false, 0).unwrap();
        policy.attach(Slot::Sidebar, 3, true, true).unwrap();
        policy.attach(Slot::Sidebar, 4, true, false).unwrap();
        assert_eq!(
            policy.attach(Slot::Sidebar, 3, true, false),
            Err(Error::Occupied)
        );
        assert_eq!(policy.sidebar_panels(), [2, 3, 4]);
        assert_eq!(policy.sidebar_active(), Some(2));
        let hidden = sidebar(&policy, 0);
        assert_eq!((hidden.owner, hidden.reason), (2, Some(Reason::UserHidden)));
        assert!(!hidden.detached);
        // Any tab's owner shows, hides or detaches the whole slot.
        policy.set_preferred(Slot::Sidebar, 4, true, 0).unwrap();
        policy.set_active(3).unwrap();
        let shown = sidebar(&policy, 0);
        assert_eq!((shown.owner, shown.effective), (3, true));
        assert_eq!(policy.set_active(9), Err(Error::MissingOwner));
        policy.set_detached(4, true).unwrap();
        assert!(sidebar(&policy, 0).detached);
        policy.set_detached(2, false).unwrap();
        // Order: listed owners first, the rest after them in their order.
        policy.order_sidebar(&[4, 2]);
        assert_eq!(policy.sidebar_panels(), [4, 2, 3]);
        policy.order_sidebar(&[2, 3, 4]);
        // The current tab removed: the next one takes over, the last one's
        // predecessor when it was the last; a background tab leaves it.
        policy.remove_owner(3);
        assert_eq!(policy.sidebar_active(), Some(4));
        policy.attach(Slot::Sidebar, 5, false, true).unwrap();
        policy.remove_owner(5);
        assert_eq!(policy.sidebar_active(), Some(4));
        policy.remove_owner(4);
        assert_eq!(policy.sidebar_active(), Some(2));
        assert!(sidebar(&policy, 0).effective, "the slot keeps its state");
        policy.remove_owner(2);
        assert_eq!(policy.sidebar_active(), None);
        assert!(policy.evaluate(screen(), 0).unwrap().sidebar.is_none());
        // Empty again: the next panel brings the slot's state.
        policy.attach(Slot::Sidebar, 6, false, false).unwrap();
        assert_eq!(sidebar(&policy, 0).reason, Some(Reason::UserHidden));
    }

    #[test]
    fn only_the_current_tabs_locks_hold_the_popped_out_sidebar() {
        let mut policy = attached();
        policy.attach(Slot::Sidebar, 3, true, false).unwrap();
        let focused = Locks {
            focused: true,
            ..Locks::default()
        };
        policy.set_locks(Slot::Sidebar, 3, focused).unwrap();
        policy.set_fullscreen(true, 0);
        policy.pointer(1919.0, 1920.0, 0);
        policy.tick(4000);
        assert!(
            !sidebar(&policy, 4000).effective,
            "a background tab's lock does not hold it"
        );
        policy.set_active(3).unwrap();
        policy.pointer(1919.0, 1920.0, 5000);
        policy.tick(20_000);
        assert!(sidebar(&policy, 20_000).effective);
        assert_eq!(
            policy.set_locks(Slot::Sidebar, 9, focused),
            Err(Error::MissingOwner)
        );
    }
}
