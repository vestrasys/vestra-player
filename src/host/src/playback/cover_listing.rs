//! Cover art listing off the backend thread.
//!
//! A leased load gives libmpv no path, so the host lists the pictures beside
//! an audio file itself (`source_backend::cover_art`) and passes them as
//! `cover-art-files`. Listing reads the whole folder, which on a huge network
//! folder can take seconds; the backend thread (libmpv's events, every
//! playback request) must not wait for that. The folder is read on its own
//! thread: when the pictures come within [`WAIT`] the load carries them as
//! before; otherwise the file loads without them and, once they come, the
//! same pictures are added to that load with `video-add` (`albumart`),
//! selected as `audio-display=embedded-first` would have: the first one that
//! opens, unless the file's own picture is already shown; one at a time, so
//! they keep their order. Either way the same pictures, in the same order,
//! become the file's cover art tracks.
//!
//! At most [`MAX_RUNNING`] folders are read at once. Loads from a folder
//! already being read (the next song of the same album) share that read;
//! other folders queue for a free reader, so a file waits for its cover art
//! rather than going without it.
#![forbid(unsafe_code)]

use rsmpv::{Mpv, Node};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

/// How long a load waits for its pictures before it goes on without them.
pub const WAIT: Duration = Duration::from_millis(200);
/// Folders read at once. A folder that never answers keeps its reader;
/// further folders queue until a reader is free.
pub const MAX_RUNNING: usize = 4;

/// The pictures of one load, as the paths given to libmpv.
pub type Pictures = Vec<String>;

/// Pictures that did not come within the wait.
pub struct Listing(mpsc::Receiver<Pictures>);

/// What a load gets from [`Lister::list`].
pub enum Covers {
    /// The pictures, in time for the load.
    Now(Pictures),
    /// Still listing: the load goes on without them ([`Late`]).
    Later(Listing),
}

/// Ranks a folder's files into one load's pictures.
type Waiter = Box<dyn FnOnce(&[PathBuf]) + Send>;
/// Reads a folder's files (`source_backend::folder_files`; tests fake it).
type Read = Arc<dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync>;

/// The folder readers, shared by the loads of one player.
#[derive(Clone)]
pub struct Lister {
    shared: Arc<Mutex<Shared>>,
    read: Read,
}
#[derive(Default)]
struct Shared {
    /// Reader threads alive.
    running: usize,
    /// Folders waiting for a reader, oldest first.
    queue: VecDeque<PathBuf>,
    /// The loads waiting for each folder being read or queued.
    waiting: HashMap<PathBuf, Vec<Waiter>>,
}
impl Lister {
    pub fn new(read: impl Fn(&Path) -> Vec<PathBuf> + Send + Sync + 'static) -> Self {
        Self {
            shared: Arc::default(),
            read: Arc::new(read),
        }
    }
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The pictures `rank` picks from `folder`'s files, waiting up to `wait`
    /// (`None`: not at all, tests).
    pub fn list(
        &self,
        folder: PathBuf,
        wait: Option<Duration>,
        rank: impl FnOnce(&[PathBuf]) -> Pictures + Send + 'static,
    ) -> Covers {
        let (send, receive) = mpsc::sync_channel(1);
        let waiter: Waiter = Box::new(move |files| {
            let _ = send.send(rank(files));
        });
        let start = {
            let mut shared = self.lock();
            if let Some(waiters) = shared.waiting.get_mut(&folder) {
                // Being read or queued: share that read.
                waiters.push(waiter);
            } else {
                shared.waiting.insert(folder.clone(), vec![waiter]);
                shared.queue.push_back(folder);
            }
            let mut start = Vec::new();
            while shared.running < MAX_RUNNING
                && let Some(next) = shared.queue.pop_front()
            {
                shared.running += 1;
                start.push(next);
            }
            if !shared.queue.is_empty() {
                diag!(
                    "cover art: {MAX_RUNNING} folders still being read; {} queued",
                    shared.queue.len()
                );
            }
            start
        };
        for folder in start {
            self.spawn(folder);
        }
        match wait.map(|wait| receive.recv_timeout(wait)) {
            Some(Ok(pictures)) => Covers::Now(pictures),
            Some(Err(mpsc::RecvTimeoutError::Disconnected)) => Covers::Now(Vec::new()),
            Some(Err(mpsc::RecvTimeoutError::Timeout)) | None => Covers::Later(Listing(receive)),
        }
    }
    /// A reader for `folder`, which holds one of the `running` places.
    fn spawn(&self, folder: PathBuf) {
        let lister = self.clone();
        let read = folder.clone();
        let spawned = std::thread::Builder::new()
            .name("cover-art".into())
            .spawn(move || lister.work(read));
        if let Err(error) = spawned {
            diag!("cover art: no reader thread ({error})");
            let mut shared = self.lock();
            shared.running -= 1;
            // Its loads go without (their receivers see the senders dropped).
            shared.waiting.remove(&folder);
        }
    }
    /// Reads `folder`, answers its loads, then the queued folders, until
    /// none is left.
    fn work(&self, mut folder: PathBuf) {
        loop {
            let slot = Slot {
                lister: self,
                folder: Some(folder.clone()),
            };
            let files = (self.read)(&folder);
            let waiters = self.lock().waiting.remove(&folder).unwrap_or_default();
            for waiter in waiters {
                waiter(&files);
            }
            slot.done();
            let next = {
                let mut shared = self.lock();
                let next = shared.queue.pop_front();
                if next.is_none() {
                    shared.running -= 1;
                }
                next
            };
            match next {
                Some(next) => folder = next,
                None => return,
            }
        }
    }
}
/// A reader's place while it reads one folder. Dropped without `done` (the
/// read or a ranking panicked): that folder's remaining loads go without,
/// and the place passes to a new reader for the next queued folder, or is
/// given back.
struct Slot<'a> {
    lister: &'a Lister,
    folder: Option<PathBuf>,
}
impl Slot<'_> {
    fn done(mut self) {
        self.folder = None;
    }
}
impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let Some(folder) = self.folder.take() else {
            return;
        };
        let next = {
            let mut shared = self.lister.lock();
            shared.waiting.remove(&folder);
            let next = shared.queue.pop_front();
            if next.is_none() {
                shared.running -= 1;
            }
            next
        };
        if let Some(next) = next {
            self.lister.spawn(next);
        }
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
/// `userdata` of the `video-add` replies for libmpv entry `entry`: a tag
/// in the top 16 bits and the entry below, so a reply still on its way for
/// an earlier entry is not taken for the current one's.
pub fn reply(entry: i64) -> u64 {
    const TAG: u64 = 0xc0fe << 48;
    TAG | (entry as u64 & ((1 << 48) - 1))
}

/// What adding the pictures needs of libmpv (tests use a fake).
pub trait Player {
    /// `audio-display=no` (a backdrop is mounted): no cover art at all.
    fn cover_art_off(&self) -> bool;
    /// A video track (the file's own picture) is selected.
    fn video_selected(&self) -> bool;
    /// Sends one `video-add` asynchronously, its reply carrying `userdata`;
    /// `false` when libmpv refused it at once.
    fn send(&self, userdata: u64, command: &Node) -> bool;
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
    fn send(&self, userdata: u64, command: &Node) -> bool {
        self.command_node_async(userdata, command)
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
    /// libmpv answered a `video-add` of this entry (any reply with
    /// [`reply`]`(entry)`; a picture that does not open is left out, as at
    /// load).
    pub fn replied(&mut self, userdata: u64) {
        if let State::Adding { waiting, .. } = &mut self.state
            && userdata == reply(self.entry)
        {
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
            // A backdrop mounted after the load but before the pictures came
            // (`audio-display=no`, `cover-art-auto=no`) leaves the whole set
            // out: a load after the switch gets no cover art either. One
            // mounted while pictures are being added stops the rest (below);
            // those already added stay, as for a file loaded before it.
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
        if player.cover_art_off() {
            return Step::Drop;
        }
        // Selected as at load: the first picture that opens, unless a video
        // track (the file's own picture, or an earlier one) already shows.
        while let Some(picture) = next.pop_front() {
            if player.send(
                reply(self.entry),
                &command(&picture, !player.video_selected()),
            ) {
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    fn pictures(names: &[&str]) -> Pictures {
        names.iter().map(|name| (*name).to_owned()).collect()
    }
    fn names(files: &[PathBuf]) -> Pictures {
        files
            .iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect()
    }
    /// A folder read that answers with the folder's cover.jpg once released.
    fn gated() -> (Lister, mpsc::Sender<()>, Arc<AtomicUsize>) {
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = reads.clone();
        let lister = Lister::new(move |folder| {
            counted.fetch_add(1, Ordering::AcqRel);
            let _ = gate.lock().unwrap().recv();
            vec![folder.join("cover.jpg")]
        });
        (lister, release, reads)
    }
    fn answer(covers: Covers) -> Pictures {
        match covers {
            Covers::Now(found) => found,
            Covers::Later(Listing(receive)) => receive
                .recv_timeout(Duration::from_secs(5))
                .expect("the pictures come"),
        }
    }
    fn running(lister: &Lister) -> usize {
        lister.lock().running
    }
    fn settled(lister: &Lister) {
        let end = Instant::now() + Duration::from_secs(5);
        while running(lister) != 0 {
            assert!(Instant::now() < end, "readers still running");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// libmpv as `Late` sees it: the pictures sent, in order.
    #[derive(Default)]
    struct Fake {
        own_picture: std::cell::Cell<bool>,
        off: std::cell::Cell<bool>,
        refuse: Option<String>,
        sent: std::cell::RefCell<Vec<(String, String)>>,
        userdata: std::cell::RefCell<Vec<u64>>,
    }
    impl Player for Fake {
        fn cover_art_off(&self) -> bool {
            self.off.get()
        }
        fn video_selected(&self) -> bool {
            self.own_picture.get()
        }
        fn send(&self, userdata: u64, command: &Node) -> bool {
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
            self.userdata.borrow_mut().push(userdata);
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
    /// Entry 1, its pictures listed, waiting for the load.
    fn listed(found: &[&str]) -> Late {
        let found = pictures(found);
        let lister = Lister::new(|_| Vec::new());
        let Covers::Later(listing) = lister.list("dir".into(), None, move |_| found) else {
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
        let lister = Lister::new(|folder| vec![folder.join("a.jpg"), folder.join("b.txt")]);
        let covers = lister.list("dir".into(), Some(Duration::from_secs(5)), |files| {
            names(&files[..1])
        });
        let Covers::Now(found) = covers else {
            panic!("a quick listing answers within the wait");
        };
        assert_eq!(found, names(&[PathBuf::from("dir").join("a.jpg")]));
        settled(&lister);
    }

    #[test]
    fn a_slow_listing_does_not_hold_the_load_and_answers_later() {
        let (lister, release, _) = gated();
        let started = Instant::now();
        let covers = lister.list("dir".into(), Some(Duration::from_millis(20)), |_| {
            pictures(&["song.jpg", "cover.jpg"])
        });
        let Covers::Later(listing) = covers else {
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
        assert_eq!(*fake.userdata.borrow(), [reply(7)]);
        // A reply for another entry (an earlier load's, still on its way).
        late.replied(reply(6));
        assert_eq!(
            late.poll(true, true, &fake),
            Step::Wait,
            "another entry's reply"
        );
        late.replied(reply(7));
        assert_eq!(late.poll(true, true, &fake), Step::Wait);
        late.replied(reply(7));
        assert_eq!(late.poll(true, true, &fake), Step::Drop);
        assert_eq!(
            sent(&fake),
            [pair("song.jpg", "select"), pair("cover.jpg", "auto")]
        );
        settled(&lister);
    }

    #[test]
    fn replies_name_their_entry() {
        assert_ne!(reply(1), reply(2));
        assert_eq!(reply(5) & ((1 << 48) - 1), 5);
        assert_ne!(reply(1), 0, "not the userdata of the host's other requests");
    }

    #[test]
    fn a_superseded_load_gets_nothing() {
        let lister = Lister::new(|_| Vec::new());
        let covers = lister.list("dir".into(), None, |_| pictures(&["cover.jpg"]));
        let Covers::Later(listing) = covers else {
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

    /// Four folders that do not answer: a fifth queues and gets its pictures
    /// once a reader is free, never an empty list.
    #[test]
    fn folders_past_the_readers_queue_instead_of_going_without() {
        let (lister, release, reads) = gated();
        let late = (0..=MAX_RUNNING)
            .map(|n| lister.list(format!("dir{n}").into(), None, names))
            .collect::<Vec<_>>();
        assert!(late.iter().all(|covers| matches!(covers, Covers::Later(_))));
        let end = Instant::now() + Duration::from_secs(5);
        while reads.load(Ordering::Acquire) < MAX_RUNNING {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(running(&lister), MAX_RUNNING);
        assert_eq!(lister.lock().queue.len(), 1, "the fifth folder waits");
        for _ in 0..=MAX_RUNNING {
            release.send(()).unwrap();
        }
        for (n, covers) in late.into_iter().enumerate() {
            let folder = PathBuf::from(format!("dir{n}"));
            assert_eq!(answer(covers), names(&[folder.join("cover.jpg")]));
        }
        assert_eq!(reads.load(Ordering::Acquire), MAX_RUNNING + 1);
        settled(&lister);
    }

    /// The next song of the same album: one read of the folder serves both.
    #[test]
    fn loads_from_a_folder_being_read_share_the_read() {
        let (lister, release, reads) = gated();
        let first = lister.list("album".into(), None, names);
        let second = lister.list("album".into(), None, |files| {
            names(files).into_iter().rev().collect()
        });
        release.send(()).unwrap();
        let cover = names(&[PathBuf::from("album").join("cover.jpg")]);
        assert_eq!(answer(first), cover);
        assert_eq!(answer(second), cover);
        assert_eq!(reads.load(Ordering::Acquire), 1);
        settled(&lister);
    }

    /// A read that panics gives its place back (or passes it to the next
    /// queued folder); its loads go without, the others are not held up.
    #[test]
    fn a_reader_that_panics_gives_its_place_back() {
        let lister = Lister::new(|folder| {
            assert!(folder != Path::new("bad"), "unreadable");
            vec![folder.join("cover.jpg")]
        });
        let Covers::Later(Listing(bad)) = lister.list("bad".into(), None, names) else {
            panic!("late");
        };
        assert_eq!(
            bad.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        settled(&lister);
        // Every place is free again.
        let good = (0..MAX_RUNNING)
            .map(|n| lister.list(format!("dir{n}").into(), None, names))
            .collect::<Vec<_>>();
        for covers in good {
            assert_eq!(answer(covers).len(), 1);
        }
        settled(&lister);

        // A panic with a folder queued behind it hands the place on.
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let lister = Lister::new(move |folder| {
            let _ = gate.lock().unwrap().recv();
            assert!(folder != Path::new("bad"), "unreadable");
            vec![folder.join("cover.jpg")]
        });
        let held = (1..MAX_RUNNING)
            .map(|n| lister.list(format!("dir{n}").into(), None, names))
            .collect::<Vec<_>>();
        let bad = lister.list("bad".into(), None, names);
        let queued = lister.list("queued".into(), None, names);
        assert_eq!(lister.lock().queue.len(), 1);
        for _ in 0..=MAX_RUNNING {
            release.send(()).unwrap();
        }
        assert_eq!(
            answer(queued),
            names(&[PathBuf::from("queued").join("cover.jpg")])
        );
        let Covers::Later(Listing(bad)) = bad else {
            panic!("late");
        };
        assert!(bad.recv_timeout(Duration::from_secs(5)).is_err());
        for covers in held {
            assert_eq!(answer(covers).len(), 1);
        }
        settled(&lister);
    }

    #[test]
    fn the_files_own_picture_stays_selected_and_a_backdrop_gets_none() {
        let own = Fake {
            own_picture: true.into(),
            ..Fake::default()
        };
        let mut late = listed(&["a.jpg", "b.jpg"]);
        assert_eq!(late.poll(true, true, &own), Step::Wait);
        late.replied(reply(1));
        assert_eq!(late.poll(true, true, &own), Step::Wait);
        late.replied(reply(1));
        assert_eq!(late.poll(true, true, &own), Step::Drop);
        assert_eq!(sent(&own), [pair("a.jpg", "auto"), pair("b.jpg", "auto")]);
        // A backdrop mounted before the pictures came: none of them.
        let off = Fake {
            off: true.into(),
            ..Fake::default()
        };
        assert_eq!(listed(&["a.jpg"]).poll(true, true, &off), Step::Drop);
        assert!(sent(&off).is_empty());
        // Mounted while they are added: the rest are left out.
        let fake = Fake::default();
        let mut late = listed(&["a.jpg", "b.jpg"]);
        assert_eq!(late.poll(true, true, &fake), Step::Wait);
        fake.off.set(true);
        late.replied(reply(1));
        assert_eq!(late.poll(true, true, &fake), Step::Drop);
        assert_eq!(sent(&fake), [pair("a.jpg", "select")]);
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
        late.replied(reply(1));
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
