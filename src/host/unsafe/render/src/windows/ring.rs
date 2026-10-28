//! Who may use which texture of the ring (pure; tested below).
//!
//! The render thread draws into one texture while windows show another:
//!
//! - a window claims the newest published texture for each draw that shows
//!   it and releases it once that draw is submitted (one draw; never across
//!   draws); the GPU side of the claim is the `done` fence the draw signals;
//! - the render thread draws only into a texture that is neither the newest
//!   published one (a window may claim it at any moment) nor claimed;
//! - a resize makes a new ring (a new generation). Claims and the newest
//!   frame of the old one stay valid until released or replaced; nothing is
//!   drawn into the old textures again.

/// Frames in flight: one being drawn, one being shown, one spare.
pub const RING: usize = 3;

/// A texture of one generation of the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub generation: u64,
    pub index: usize,
}

/// The newest published frame.
#[derive(Clone, Debug)]
pub struct Frame<T> {
    pub slot: Slot,
    pub sequence: u64,
    /// What the window needs to use the slot (the ring's textures).
    pub payload: T,
}

#[derive(Debug)]
pub struct Ring<T> {
    generation: u64,
    next: usize,
    sequence: u64,
    latest: Option<Frame<T>>,
    /// One entry per claim (a slot may be claimed by several windows).
    claims: Vec<Slot>,
}
impl<T> Default for Ring<T> {
    fn default() -> Self {
        Self { generation: 0, next: 0, sequence: 0, latest: None, claims: Vec::new() }
    }
}
impl<T: Clone> Ring<T> {
    /// New textures (a resize): returns their generation.
    pub fn renew(&mut self) -> u64 {
        self.generation += 1;
        self.next = 0;
        self.generation
    }
    fn busy(&self, slot: Slot) -> bool {
        self.latest.as_ref().is_some_and(|latest| latest.slot == slot)
            || self.claims.contains(&slot)
    }
    /// The texture of the current generation to draw into next, in turn;
    /// `None` while every one is the newest frame or claimed.
    pub fn target(&mut self) -> Option<usize> {
        let generation = self.generation;
        let index = (0..RING)
            .map(|step| (self.next + step) % RING)
            .find(|&index| !self.busy(Slot { generation, index }))?;
        self.next = (index + 1) % RING;
        Some(index)
    }
    /// `index` of the current generation holds a finished frame: it becomes
    /// the newest one. Returns its number (from 1).
    pub fn publish(&mut self, index: usize, payload: T) -> u64 {
        self.sequence += 1;
        let slot = Slot { generation: self.generation, index };
        self.latest = Some(Frame { slot, sequence: self.sequence, payload });
        self.sequence
    }
    /// A window takes the newest frame for one draw.
    pub fn claim(&mut self) -> Option<Frame<T>> {
        let latest = self.latest.clone()?;
        self.claims.push(latest.slot);
        Some(latest)
    }
    /// The window's draw that showed `slot` was submitted.
    pub fn release(&mut self, slot: Slot) {
        if let Some(at) = self.claims.iter().position(|claimed| *claimed == slot) {
            self.claims.swap_remove(at);
        }
    }
    pub fn latest(&self) -> Option<&Frame<T>> {
        self.latest.as_ref()
    }
    /// The current generation (textures drawn into from now on).
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Frames published so far.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_in_turn_and_never_into_the_newest_frame() {
        let mut ring = Ring::<()>::default();
        ring.renew();
        assert_eq!(ring.target(), Some(0));
        assert_eq!(ring.publish(0, ()), 1);
        assert_eq!(ring.target(), Some(1));
        ring.publish(1, ());
        assert_eq!(ring.target(), Some(2));
        ring.publish(2, ());
        // 2 is the newest: 0 and 1 are free again.
        assert_eq!(ring.target(), Some(0));
        ring.publish(0, ());
        assert_eq!(ring.target(), Some(1));
        assert_eq!(ring.sequence(), 4);
    }

    #[test]
    fn a_claimed_texture_is_skipped_until_released() {
        let mut ring = Ring::<()>::default();
        ring.renew();
        let index = ring.target().unwrap();
        ring.publish(index, ());
        let claim = ring.claim().unwrap();
        assert_eq!((claim.slot.index, claim.sequence), (0, 1));
        // The window still copies 0; the renderer publishes 1 and 2.
        let index = ring.target().unwrap();
        ring.publish(index, ());
        assert_eq!(ring.target(), Some(2));
        ring.publish(2, ());
        // 0 claimed, 2 newest: only 1 is free.
        assert_eq!(ring.target(), Some(1));
        ring.publish(1, ());
        // 0 claimed, 1 newest, 2 free.
        let second = ring.claim().unwrap();
        assert_eq!(second.slot.index, 1);
        // 0 and 1 claimed by two windows, 1 also newest: only 2.
        assert_eq!(ring.target(), Some(2));
        ring.publish(2, ());
        // 0 and 1 claimed, 2 newest: nothing to draw into.
        assert_eq!(ring.target(), None);
        ring.release(claim.slot);
        assert_eq!(ring.target(), Some(0));
    }

    #[test]
    fn the_same_slot_claimed_twice_needs_two_releases() {
        let mut ring = Ring::<()>::default();
        ring.renew();
        ring.publish(0, ());
        let (a, b) = (ring.claim().unwrap(), ring.claim().unwrap());
        ring.publish(1, ());
        ring.release(a.slot);
        // 0 is still claimed by the second window, 1 is the newest.
        assert_eq!(ring.target(), Some(2));
        ring.publish(2, ());
        assert_eq!(ring.target(), Some(1));
        ring.publish(1, ());
        assert_eq!(ring.target(), Some(2));
        ring.release(b.slot);
        ring.publish(2, ());
        assert_eq!(ring.target(), Some(0));
    }

    #[test]
    fn a_new_generation_ignores_the_old_claims_and_keeps_the_newest_frame() {
        let mut ring = Ring::default();
        ring.renew();
        ring.publish(0, "old");
        let claim = ring.claim().unwrap();
        assert_eq!(ring.renew(), 2);
        // The old frame is still the newest until a new one is published.
        assert_eq!(ring.latest().map(|f| (f.slot, f.payload)), Some((claim.slot, "old")));
        // Old claims and the old newest frame do not block the new textures.
        assert_eq!(ring.target(), Some(0));
        assert_eq!(ring.publish(0, "new"), 2);
        let latest = ring.claim().unwrap();
        assert_eq!((latest.slot, latest.payload), (Slot { generation: 2, index: 0 }, "new"));
        ring.release(claim.slot);
        ring.release(latest.slot);
        assert_eq!(ring.target(), Some(1));
    }

    #[test]
    fn nothing_published_nothing_to_claim() {
        let mut ring = Ring::<()>::default();
        ring.renew();
        assert!(ring.claim().is_none());
        assert_eq!(ring.target(), Some(0));
        // Drawn but not published (the publish gate is closed): drawn into
        // again next time, nothing claimable.
        assert_eq!(ring.target(), Some(1));
        assert!(ring.claim().is_none() && ring.sequence() == 0);
    }
}
