//! Cover art listing off the backend thread.
//!
//! A leased load gives libmpv no path, so the host lists the pictures beside
//! an audio file itself (`source_backend::cover_art`) and passes them as
//! `cover-art-files`. Listing reads the whole folder, which on a huge network
//! folder can take seconds; the backend thread (libmpv's events, every
//! playback request) must not wait for that. The listing runs on its own
//! thread: when it answers within [`WAIT`] the load carries the pictures as
//! before; otherwise the file loads without them and, once the listing
//! answers, the same pictures are added to that load with `video-add`
//! (`albumart`), selected as `audio-display=embedded-first` would have: the
//! first one, unless the file's own picture is already shown; one at a time,
//! so they keep their order. Either way the same pictures, in the same
//! order, become the file's cover art tracks.
#![forbid(unsafe_code)]

use rsmpv::{Mpv, Node};
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

/// How long a load waits for its listing before it goes on without it.
pub const WAIT: Duration = Duration::from_millis(200);
/// Listings running at once. A folder that never answers keeps its thread;
/// past this many such threads, later files get no listing (no cover art)
/// instead of more threads.
pub const MAX_RUNNING: usize = 4;

/// The pictures of one load, as the paths given to libmpv.
pub type Pictures = Vec<String>;

/// A listing that did not answer within the wait.
pub struct Listing(mpsc::Receiver<Pictures>);

/// What a load gets from [`list`].
pub enum Covers {
    /// The pictures, in time for the load.
    Now(Pictures),
    /// Still listing: the load goes on without them ([`Late`]).
    Later(Listing),
}

/// Lists on its own thread and waits up to `wait` (`None`: not at all,
/// tests). `running` counts the listing threads alive (see [`MAX_RUNNING`]).
pub fn list(
    running: &Arc<AtomicUsize>,
    wait: Option<Duration>,
    listing: impl FnOnce() -> Pictures + Send + 'static,
) -> Covers {
    if running.fetch_add(1, Ordering::AcqRel) >= MAX_RUNNING {
        running.fetch_sub(1, Ordering::AcqRel);
        diag!("cover art: {MAX_RUNNING} folder listings still running; no cover art");
        return Covers::Now(Vec::new());
    }
    let (send, receive) = mpsc::sync_channel(1);
    let alive = running.clone();
    let spawned = std::thread::Builder::new()
        .name("cover-art".into())
        .spawn(move || {
            let _ = send.send(listing());
            alive.fetch_sub(1, Ordering::AcqRel);
        });
    if let Err(error) = spawned {
        running.fetch_sub(1, Ordering::AcqRel);
        diag!("cover art: no listing thread ({error})");
        return Covers::Now(Vec::new());
    }
    match wait.map(|wait| receive.recv_timeout(wait)) {
        Some(Ok(pictures)) => Covers::Now(pictures),
        Some(Err(mpsc::RecvTimeoutError::Disconnected)) => Covers::Now(Vec::new()),
        Some(Err(mpsc::RecvTimeoutError::Timeout)) | None => Covers::Later(Listing(receive)),
    }
}

/// The late pictures of one libmpv entry: listing, then listed and waiting
/// for the entry's `FileLoaded` (mpv picks the tracks at load), then added
/// one at a time: each `video-add` runs on a libmpv thread of its own, so
/// the next is sent only after the previous one answered (`replied`), which
/// keeps the tracks in the listed order.
pub struct Late {
    pub entry: i64,
    state: State,
}
enum State {
    Listing(Listing),
    Listed(Pictures),
    Adding {
        next: VecDeque<String>,
        waiting: bool,
    },
}
/// What [`Late::poll`] decided.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Keep it and ask again later.
    Wait,
    /// Done or no longer wanted: drop it.
    Drop,
}
/// `userdata` of the `video-add` replies.
pub const REPLY: u64 = 0x636f_7665_7261_7274; // "coverart"

/// What adding the pictures needs of libmpv (tests use a fake).
pub trait Player {
    /// `audio-display=no` (a backdrop is mounted): no cover art at all.
    fn cover_art_off(&self) -> bool;
    /// A video track (the file's own picture) is selected.
    fn video_selected(&self) -> bool;
    /// Sends one `video-add` asynchronously (reply [`REPLY`]); `false` when
    /// libmpv refused it at once.
    fn send(&self, command: &Node) -> bool;
}
impl Player for Mpv {
    fn cover_art_off(&self) -> bool {
        self.get_property::<String>("audio-display")
            .is_ok_and(|mode| mode == "no")
    }
    fn video_selected(&self) -> bool {
        self.get_property::<String>("vid")
            .is_ok_and(|vid| vid != "no")
    }
    fn send(&self, command: &Node) -> bool {
        self.command_node_async(REPLY, command)
            .inspect_err(|error| diag!("cover art: video-add refused: {error}"))
            .is_ok()
    }
}
impl Late {
    pub fn new(entry: i64, listing: Listing) -> Self {
        Self {
            entry,
            state: State::Listing(listing),
        }
    }
    /// libmpv answered a `video-add` (any reply with [`REPLY`]; a picture
    /// that does not open is left out, as at load).
    pub fn replied(&mut self, userdata: u64) {
        if let (REPLY, State::Adding { waiting, .. }) = (userdata, &mut self.state) {
            *waiting = false;
        }
    }
    /// `loaded`: the entry is the loaded one now; `current`: it is still
    /// the load the source waits for. Adds the pictures once both the
    /// listing answered and the entry loaded, one per answer.
    pub fn poll(&mut self, loaded: bool, current: bool, player: &dyn Player) -> Step {
        if !current && !loaded {
            return Step::Drop;
        }
        if let State::Listing(Listing(receive)) = &self.state {
            match receive.try_recv() {
                Ok(pictures) => self.state = State::Listed(pictures),
                Err(mpsc::TryRecvError::Empty) => return Step::Wait,
                Err(mpsc::TryRecvError::Disconnected) => return Step::Drop,
            }
        }
        if !loaded {
            // Listed: wait for the load. Adding: the entry ended.
            return match self.state {
                State::Adding { .. } => Step::Drop,
                _ => Step::Wait,
            };
        }
        if let State::Listed(pictures) = &self.state {
            if pictures.is_empty() || player.cover_art_off() {
                return Step::Drop;
            }
            self.state = State::Adding {
                next: pictures.iter().cloned().collect(),
                waiting: false,
            };
        }
        let State::Adding { next, waiting } = &mut self.state else {
            return Step::Wait;
        };
        if *waiting {
            return Step::Wait;
        }
        // Selected as at load: the first picture that opens, unless a video
        // track (the file's own picture, or an earlier one) already shows.
        while let Some(picture) = next.pop_front() {
            if player.send(&command(&picture, !player.video_selected())) {
                *waiting = true;
                return Step::Wait;
            }
        }
        Step::Drop
    }
}

/// `video-add` of one picture as `albumart`, selected or `auto`.
pub fn command(picture: &str, select: bool) -> Node {
    let flags = if select { "select" } else { "auto" };
    Node::Map(vec![
        ("name".into(), Node::String("video-add".into())),
        ("url".into(), Node::String(picture.into())),
        ("flags".into(), Node::String(flags.into())),
        ("albumart".into(), Node::Flag(true)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn pictures(names: &[&str]) -> Pictures {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// libmpv as `Late` sees it: the pictures sent, in order.
    #[derive(Default)]
    struct Fake {
        own_picture: std::cell::Cell<bool>,
        off: bool,
        refuse: Option<String>,
        sent: std::cell::RefCell<Vec<(String, String)>>,
    }
    impl Player for Fake {
        fn cover_art_off(&self) -> bool {
            self.off
        }
        fn video_selected(&self) -> bool {
            self.own_picture.get()
        }
        fn send(&self, command: &Node) -> bool {
            let Node::Map(fields) = command else {
                panic!("named arguments")
            };
            let text = |key: &str| match fields.iter().find(|(name, _)| name == key) {
                Some((_, Node::String(value))) => value.clone(),
                _ => panic!("{key}"),
            };
            if self.refuse.as_deref() == Some(text("url").as_str()) {
                return false;
            }
            if text("flags") == "select" {
                self.own_picture.set(true);
            }
            self.sent.borrow_mut().push((text("url"), text("flags")));
            true
        }
    }
    fn sent(fake: &Fake) -> Vec<(String, String)> {
        fake.sent.borrow().clone()
    }
    fn pair(url: &str, flags: &str) -> (String, String) {
        (url.to_owned(), flags.to_owned())
    }
    fn listed(names: &[&str]) -> Late {
        let running = Arc::new(AtomicUsize::new(0));
        let names = pictures(names);
        let Covers::Later(listing) = list(&running, None, move || names) else {
            panic!("no wait: always late");
        };
        let mut late = Late::new(1, listing);
        let end = Instant::now() + Duration::from_secs(5);
        while !matches!(late.state, State::Listed(_)) {
            assert!(Instant::now() < end);
            assert_eq!(late.poll(false, true, &Fake::default()), Step::Wait);
            std::thread::sleep(Duration::from_millis(1));
        }
        late
    }

    #[test]
    fn a_quick_listing_is_in_time_for_the_load() {
        let running = Arc::new(AtomicUsize::new(0));
        let Covers::Now(found) = list(&running, Some(Duration::from_secs(5)), || {
            pictures(&["a.jpg", "cover.jpg"])
        }) else {
            panic!("a quick listing answers within the wait");
        };
        assert_eq!(found, ["a.jpg", "cover.jpg"]);
    }

    #[test]
    fn a_slow_listing_does_not_hold_the_load_and_answers_later() {
        let running = Arc::new(AtomicUsize::new(0));
        let (release, gate) = mpsc::channel::<()>();
        let started = Instant::now();
        let Covers::Later(listing) = list(&running, Some(Duration::from_millis(20)), move || {
            let _ = gate.recv();
            pictures(&["song.jpg", "cover.jpg"])
        }) else {
            panic!("a blocked listing is late");
        };
        assert!(started.elapsed() < Duration::from_secs(2));
        let mut late = Late::new(7, listing);
        let fake = Fake::default();
        assert_eq!(late.poll(false, true, &fake), Step::Wait, "listing");
        release.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        // Listed, but the entry has not loaded yet: nothing is added.
        while !matches!(late.state, State::Listed(_)) && Instant::now() < end {
            assert_eq!(late.poll(false, true, &fake), Step::Wait);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(late.state, State::Listed(_)));
        assert_eq!(late.poll(false, true, &fake), Step::Wait);
        assert!(sent(&fake).is_empty(), "nothing before the entry loaded");
        // Loaded: one picture per answer, in the listed order.
        assert_eq!(late.poll(true, true, &fake), Step::Wait);
        assert_eq!(late.poll(true, true, &fake), Step::Wait, "no answer yet");
        assert_eq!(sent(&fake), [pair("song.jpg", "select")]);
        late.replied(REPLY + 1);
        assert_eq!(late.poll(true, true, &fake), Step::Wait, "another reply");
        late.replied(REPLY);
        assert_eq!(late.poll(true, true, &fake), Step::Wait);
        late.replied(REPLY);
        assert_eq!(late.poll(true, true, &fake), Step::Drop);
        assert_eq!(
            sent(&fake),
            [pair("song.jpg", "select"), pair("cover.jpg", "auto")]
        );
        // The thread is gone once it answered.
        while running.load(Ordering::Acquire) != 0 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(running.load(Ordering::Acquire), 0);
    }

    #[test]
    fn a_superseded_load_gets_nothing() {
        let running = Arc::new(AtomicUsize::new(0));
        let Covers::Later(listing) = list(&running, None, || pictures(&["cover.jpg"])) else {
            panic!("no wait: always late");
        };
        let mut late = Late::new(1, listing);
        let fake = Fake::default();
        assert_eq!(
            late.poll(false, false, &fake),
            Step::Drop,
            "another file was opened meanwhile"
        );
        assert!(sent(&fake).is_empty());
    }

    #[test]
    fn listings_that_never_answer_are_bounded() {
        let running = Arc::new(AtomicUsize::new(0));
        let (release, gate) = mpsc::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(gate));
        let mut held = Vec::new();
        for _ in 0..MAX_RUNNING {
            let gate = gate.clone();
            let covers = list(&running, None, move || {
                let _ = gate.lock().unwrap().recv();
                Vec::new()
            });
            assert!(matches!(covers, Covers::Later(_)));
            held.push(covers);
        }
        let covers = list(&running, None, || pictures(&["cover.jpg"]));
        assert!(matches!(covers, Covers::Now(ref found) if found.is_empty()));
        assert_eq!(running.load(Ordering::Acquire), MAX_RUNNING);
        for _ in 0..MAX_RUNNING {
            release.send(()).unwrap();
        }
        drop(held);
    }

    #[test]
    fn the_files_own_picture_stays_selected_and_a_backdrop_gets_none() {
        let own = Fake {
            own_picture: true.into(),
            ..Fake::default()
        };
        let mut late = listed(&["a.jpg", "b.jpg"]);
        assert_eq!(late.poll(true, true, &own), Step::Wait);
        late.replied(REPLY);
        assert_eq!(late.poll(true, true, &own), Step::Wait);
        late.replied(REPLY);
        assert_eq!(late.poll(true, true, &own), Step::Drop);
        assert_eq!(sent(&own), [pair("a.jpg", "auto"), pair("b.jpg", "auto")]);
        let off = Fake {
            off: true,
            ..Fake::default()
        };
        assert_eq!(listed(&["a.jpg"]).poll(true, true, &off), Step::Drop);
        assert!(sent(&off).is_empty());
    }

    #[test]
    fn a_refused_picture_is_skipped_and_an_ended_entry_gets_no_more() {
        let fake = Fake {
            refuse: Some("a.jpg".into()),
            ..Fake::default()
        };
        let mut late = listed(&["a.jpg", "b.jpg", "c.jpg"]);
        assert_eq!(late.poll(true, true, &fake), Step::Wait);
        // The first that opens is selected, as at load.
        assert_eq!(sent(&fake), [pair("b.jpg", "select")]);
        late.replied(REPLY);
        // The entry ended before the last one.
        assert_eq!(late.poll(false, false, &fake), Step::Drop);
        assert_eq!(sent(&fake), [pair("b.jpg", "select")]);
    }

    #[test]
    fn a_picture_is_added_as_albumart_by_named_arguments() {
        let named = |select: bool| match command("song.jpg", select) {
            Node::Map(fields) => fields,
            _ => panic!("named arguments"),
        };
        let text = |value: &str| Node::String(value.into());
        assert_eq!(
            named(true),
            [
                ("name".into(), text("video-add")),
                ("url".into(), text("song.jpg")),
                ("flags".into(), text("select")),
                ("albumart".into(), Node::Flag(true)),
            ]
        );
        assert!(named(false).contains(&("flags".into(), text("auto"))));
    }
}
