//! Experimental finite shell visibility policy; final public API may differ.
//! No OS calls, clocks, resizing, plugin identity, or layout description protocol.
//! The host supplies monotonic elapsed milliseconds and usable logical pixels.
#![forbid(unsafe_code)]

use serde::Serialize;

const IDLE_MS: u64 = 3000;
/// G-01: the fullscreen sidebar pops out when the pointer comes this close to
/// the right edge (Vestra value, to calibrate against Q-13).
pub const EDGE_PX: f64 = 4.0;
/// G-01: the popped-out sidebar collapses this long after the pointer left it
/// (Vestra value, Q-13).
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
    /// Future idle transition only; absent while suppressed or already idle.
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
    detached: bool,
    locks: Locks,
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

pub struct Policy {
    bottom: Option<Attachment>,
    sidebar: Option<Attachment>,
    fullscreen: bool,
    last_activity_ms: u64,
    menu_open: bool,
    dialog_open: bool,
    sidebar_user_width: f64,
    /// G-04: the pointer is over the panel (host-owned, never cleared by a
    /// plugin's `setInteraction`), by slot: bottom, sidebar.
    hover: [bool; 2],
    overlay: Overlay,
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

    pub fn set_detached(&mut self, owner: u64, detached: bool) -> Result<()> {
        self.owned_mut(Slot::Sidebar, owner)?.detached = detached;
        self.overlay = Overlay::default();
        Ok(())
    }

    fn slot_mut(&mut self, slot: Slot) -> &mut Option<Attachment> {
        match slot {
            Slot::Bottom => &mut self.bottom,
            Slot::Sidebar => &mut self.sidebar,
        }
    }

    fn owned_mut(&mut self, slot: Slot, owner: u64) -> Result<&mut Attachment> {
        self.slot_mut(slot)
            .as_mut()
            .filter(|attachment| attachment.owner == owner)
            .ok_or(Error::MissingOwner)
    }

    pub fn attach(
        &mut self,
        slot: Slot,
        owner: u64,
        preferred: bool,
        detached: bool,
    ) -> Result<()> {
        let attachment = self.slot_mut(slot);
        if attachment.is_some() {
            return Err(Error::Occupied);
        }
        *attachment = Some(Attachment {
            owner,
            preferred,
            detached: detached && slot == Slot::Sidebar,
            locks: Locks::default(),
        });
        Ok(())
    }

    pub fn remove_owner(&mut self, owner: u64) {
        if self
            .bottom
            .is_some_and(|attachment| attachment.owner == owner)
        {
            self.bottom = None;
        }
        if self
            .sidebar
            .is_some_and(|attachment| attachment.owner == owner)
        {
            self.sidebar = None;
            self.overlay = Overlay::default();
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
        let attachment = self.owned_mut(slot, owner)?;
        let shown = !attachment.preferred && preferred;
        attachment.preferred = preferred;
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

    /// Setters do not invent timestamps. To give a released gesture a fresh idle
    /// period, the host first calls activity(release_now), then clears its lock.
    pub fn set_locks(&mut self, slot: Slot, owner: u64, locks: Locks) -> Result<()> {
        self.owned_mut(slot, owner)?.locks = locks;
        Ok(())
    }

    /// Repeated observation of the same mode is a no-op; entering reveals the bar.
    pub fn set_fullscreen(&mut self, fullscreen: bool, now_ms: u64) {
        if self.fullscreen == fullscreen {
            return;
        }
        self.fullscreen = fullscreen;
        // Entering fullscreen starts collapsed; leaving forgets the overlay.
        self.overlay = Overlay::default();
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

    /// Apply the timed transitions (the popped-out sidebar collapsing). The
    /// host calls this every tick before `evaluate`.
    pub fn tick(&mut self, now_ms: u64) {
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
                .is_some_and(|attachment| attachment.preferred && !attachment.detached)
    }

    /// The popped-out sidebar stays: pointer over it, the panel's own
    /// interaction locks, or a menu or dialog open.
    fn sidebar_locked(&self) -> bool {
        self.menu_open
            || self.dialog_open
            || self.hover[Slot::Sidebar as usize]
            || self
                .sidebar
                .is_some_and(|attachment| attachment.locks.active())
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

    pub fn set_obstructions(&mut self, menu_open: bool, dialog_open: bool) {
        self.menu_open = menu_open;
        self.dialog_open = dialog_open;
    }

    /// No activity for 3 seconds and nothing holding the bar (menu, dialog,
    /// panel interaction): the same rule that hides the fullscreen bar.
    pub fn idle(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_activity_ms) >= IDLE_MS
            && !self.menu_open
            && !self.dialog_open
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
        let recent = now_ms.saturating_sub(self.last_activity_ms) < IDLE_MS;
        let suppressed = self.menu_open || self.dialog_open || self.bottom_locked();
        let bottom = self.bottom.map(|attachment| {
            let reason = if !attachment.preferred {
                Some(Reason::UserHidden)
            } else if self.fullscreen && !recent && !suppressed {
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
        let sidebar = self.sidebar.map(|attachment| {
            let reason = if !attachment.preferred {
                Some(Reason::UserHidden)
            } else if attachment.detached {
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
                owner: attachment.owner,
                preferred: attachment.preferred,
                effective: reason.is_none(),
                reason,
                overlay: self.fullscreen && !attachment.detached,
                detached: attachment.detached,
            }
        });
        let next_deadline_ms = if self.fullscreen
            && !suppressed
            && recent
            && self.bottom.is_some_and(|attachment| attachment.preferred)
        {
            self.last_activity_ms
                .checked_add(IDLE_MS)
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
    fn bar_hides_at_three_seconds_without_changing_preference_and_activity_reveals() {
        let mut policy = attached();
        policy.set_fullscreen(true, 100);
        let before = policy.evaluate(roomy(), 3099).unwrap();
        assert!(before.bottom.unwrap().effective);
        assert!(before.bottom.unwrap().overlay);
        assert_eq!(before.next_deadline_ms, Some(3100));
        let idle = policy.evaluate(roomy(), 3100).unwrap();
        assert_eq!(idle.bottom.unwrap().reason, Some(Reason::Idle));
        assert!(idle.bottom.unwrap().preferred);
        assert_eq!(idle.next_deadline_ms, None);
        policy.activity(5000);
        assert!(
            policy
                .evaluate(roomy(), 5000)
                .unwrap()
                .bottom
                .unwrap()
                .effective
        );
        policy.set_preferred(Slot::Bottom, 1, false, 0).unwrap();
        policy.activity(6000);
        let hidden = policy.evaluate(roomy(), 6000).unwrap();
        assert_eq!(hidden.bottom.unwrap().reason, Some(Reason::UserHidden));
        assert_eq!(hidden.next_deadline_ms, None);
        policy.set_fullscreen(false, 7000);
        assert!(
            !policy
                .evaluate(roomy(), 7000)
                .unwrap()
                .bottom
                .unwrap()
                .overlay
        );
    }

    #[test]
    fn each_bar_lock_and_global_obstruction_suppresses_idle() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        for locks in [
            Locks {
                focused: true,
                ..Locks::default()
            },
            Locks {
                dragging: true,
                ..Locks::default()
            },
            Locks {
                popup_open: true,
                ..Locks::default()
            },
        ] {
            policy.set_locks(Slot::Bottom, 1, locks).unwrap();
            let snapshot = policy.evaluate(roomy(), 9000).unwrap();
            assert!(snapshot.bottom.unwrap().effective);
            assert_eq!(snapshot.next_deadline_ms, None);
        }
        policy.set_locks(Slot::Bottom, 1, Locks::default()).unwrap();
        for (menu, dialog) in [(true, false), (false, true)] {
            policy.set_obstructions(menu, dialog);
            assert!(
                policy
                    .evaluate(roomy(), 9000)
                    .unwrap()
                    .bottom
                    .unwrap()
                    .effective
            );
        }
        policy.set_obstructions(false, false);
        assert_eq!(
            policy
                .evaluate(roomy(), 9000)
                .unwrap()
                .bottom
                .unwrap()
                .reason,
            Some(Reason::Idle)
        );
    }

    #[test]
    fn release_activity_starts_a_new_idle_period_but_setters_do_not_invent_time() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
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
        policy.activity(9000);
        policy.set_locks(Slot::Bottom, 1, Locks::default()).unwrap();
        let released = policy.evaluate(roomy(), 9000).unwrap();
        assert!(released.bottom.unwrap().effective);
        assert_eq!(released.next_deadline_ms, Some(12000));
        policy.set_obstructions(true, false);
        assert!(
            policy
                .evaluate(roomy(), 15000)
                .unwrap()
                .bottom
                .unwrap()
                .effective
        );
        policy.set_obstructions(false, false); // No activity: original time still applies.
        assert_eq!(
            policy
                .evaluate(roomy(), 15000)
                .unwrap()
                .bottom
                .unwrap()
                .reason,
            Some(Reason::Idle)
        );
        policy.set_obstructions(false, true);
        policy.activity(16000);
        policy.set_obstructions(false, false);
        assert_eq!(
            policy.evaluate(roomy(), 16000).unwrap().next_deadline_ms,
            Some(19000)
        );
        policy.set_preferred(Slot::Bottom, 1, false, 0).unwrap();
        policy.activity(17000);
        policy
            .set_locks(
                Slot::Bottom,
                1,
                Locks {
                    focused: true,
                    dragging: true,
                    popup_open: true,
                },
            )
            .unwrap();
        policy.set_obstructions(true, true);
        let hidden = policy.evaluate(roomy(), 17000).unwrap();
        assert_eq!(hidden.bottom.unwrap().reason, Some(Reason::UserHidden));
        assert!(!hidden.bottom.unwrap().effective);
        assert_eq!(hidden.next_deadline_ms, None);
        policy.set_fullscreen(false, 17000);
        assert!(
            policy
                .evaluate(roomy(), 17000)
                .unwrap()
                .sidebar
                .unwrap()
                .effective
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

    #[test]
    fn pointer_over_the_bar_keeps_it_and_a_plugin_lock_write_cannot_clear_that() {
        let mut policy = attached();
        policy.set_fullscreen(true, 0);
        policy.set_hover(Slot::Bottom, true);
        // A plugin's setInteraction replaces its own locks only.
        policy.set_locks(Slot::Bottom, 1, Locks::default()).unwrap();
        let held = policy.evaluate(roomy(), 10_000).unwrap();
        assert!(held.bottom.unwrap().effective);
        assert_eq!(held.next_deadline_ms, None);
        assert!(
            !policy.idle(10_000),
            "the pointer stays visible over the bar"
        );
        // Moving off the bar: the host reports activity, then the hover ends.
        policy.activity(10_000);
        policy.set_hover(Slot::Bottom, false);
        assert_eq!(
            policy.evaluate(roomy(), 12_999).unwrap().next_deadline_ms,
            Some(13_000)
        );
        assert_eq!(
            policy
                .evaluate(roomy(), 13_000)
                .unwrap()
                .bottom
                .unwrap()
                .reason,
            Some(Reason::Idle)
        );
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
        let snapshot = policy.evaluate(roomy(), u64::MAX).unwrap();
        assert!(snapshot.bottom.unwrap().effective);
        assert_eq!(snapshot.next_deadline_ms, None); // Unrepresentable future time.
        policy.set_fullscreen(false, u64::MAX);
        assert!(!policy.evaluate(roomy(), u64::MAX).unwrap().fullscreen);
    }
}
