//! Cover art listing off the backend thread.
//!
//! A leased load gives libmpv no path, so the host lists the pictures beside
//! an audio file itself (`source_backend::rank_cover_art`) and passes them as
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
//! already being read or queued (the next song of the same album) share that
//! read; other folders queue for a free reader, newest first, so the file
//! playing now waits for its cover art rather than going without it, and a
//! folder whose loads all ended meanwhile is not read at all.
#![forbid(unsafe_code)]

use rsmpv::{Mpv, Node};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak, mpsc};
use std::time::Duration;

/// How long a load waits for its pictures before it goes on without them.
pub const WAIT: Duration = Duration::from_millis(200);
/// Folders read at once. A folder that never answers keeps its reader;
/// further folders queue until a reader is free.
pub const MAX_RUNNING: usize = 4;

/// The pictures of one load, as the paths given to libmpv.
pub type Pictures = Vec<String>;

/// Pictures that did not come within the wait. Dropping it (the entry
/// ended, another file was opened) withdraws the request: a folder whose
/// requests are all withdrawn before a reader is free is not read.
pub struct Listing {
    receive: mpsc::Receiver<Pictures>,
    _alive: Arc<()>,
}

/// What a load gets from [`Lister::list`].
pub enum Covers {
    /// The pictures, in time for the load.
    Now(Pictures),
    /// Still listing: the load goes on without them ([`Late`]).
    Later(Listing),
}

/// One load waiting for a folder: ranks the folder's files into its
/// pictures, while its [`Listing`] is alive.
struct Waiter {
    alive: Weak<()>,
    deliver: Deliver,
}
/// Ranks the folder's files and sends the load its pictures.
type Deliver = Box<dyn FnOnce(&[PathBuf]) + Send>;
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
    /// Folders waiting for a reader, newest request first: the file playing
    /// now is the one the user waits for, and older requests are usually
    /// withdrawn by then (quick skipping through a slow share).
    queue: VecDeque<PathBuf>,
    /// The loads waiting for each folder being read or queued.
    waiting: HashMap<PathBuf, Vec<Waiter>>,
}
impl Shared {
    /// Drops withdrawn requests everywhere, and the queued folders nobody
    /// waits for any more. Run on every request, so the queue and the
    /// waiter lists stay small while every reader is stuck (a network
    /// share that stopped answering) and no reader frees up to prune them.
    /// A folder being read keeps its entry (its reader removes it).
    fn prune(&mut self) {
        for waiters in self.waiting.values_mut() {
            waiters.retain(|waiter| waiter.alive.strong_count() > 0);
        }
        let waiting = &mut self.waiting;
        self.queue.retain(|folder| {
            let wanted = waiting
                .get(folder)
                .is_some_and(|waiters| !waiters.is_empty());
            if !wanted {
                waiting.remove(folder);
            }
            wanted
        });
    }
    /// The next queued folder someone still waits for; folders whose
    /// requests were all withdrawn are dropped unread.
    fn next_folder(&mut self) -> Option<PathBuf> {
        while let Some(folder) = self.queue.pop_front() {
            let wanted = self.waiting.get_mut(&folder).is_some_and(|waiters| {
                waiters.retain(|waiter| waiter.alive.strong_count() > 0);
                !waiters.is_empty()
            });
            if wanted {
                return Some(folder);
            }
            self.waiting.remove(&folder);
        }
        None
    }
    /// A reader finished (or could not start): the next wanted folder for
    /// it, or its place is given back.
    fn hand_on(&mut self) -> Option<PathBuf> {
        let next = self.next_folder();
        if next.is_none() {
            self.running -= 1;
        }
        next
    }
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
        let alive = Arc::new(());
        let waiter = Waiter {
            alive: Arc::downgrade(&alive),
            deliver: Box::new(move |files| {
                let _ = send.send(rank(files));
            }),
        };
        let start = {
            let mut shared = self.lock();
            shared.prune();
            if let Some(waiters) = shared.waiting.get_mut(&folder) {
                // Being read or queued: share that read; a queued folder
                // moves to the front.
                waiters.push(waiter);
                if let Some(at) = shared.queue.iter().position(|queued| *queued == folder) {
                    shared.queue.remove(at);
                    shared.queue.push_front(folder);
                }
            } else {
                shared.waiting.insert(folder.clone(), vec![waiter]);
                shared.queue.push_front(folder);
            }
            let mut start = Vec::new();
            while shared.running < MAX_RUNNING
                && let Some(next) = shared.next_folder()
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
            Some(Err(mpsc::RecvTimeoutError::Timeout)) | None => Covers::Later(Listing {
                receive,
                _alive: alive,
            }),
        }
    }
    /// A reader for `folder`, which holds one of the `running` places. When
    /// no thread can be started, that folder's loads go without (their
    /// receivers see the senders dropped) and the next queued folder is
    /// tried, until one starts or the queue is empty.
    fn spawn(&self, mut folder: PathBuf) {
        loop {
            let lister = self.clone();
            let read = folder.clone();
            let spawned = std::thread::Builder::new()
                .name("cover-art".into())
                .spawn(move || lister.work(read));
            let Err(error) = spawned else {
                return;
            };
            diag!("cover art: no reader thread ({error})");
            let mut shared = self.lock();
            shared.waiting.remove(&folder);
            match shared.hand_on() {
                Some(next) => folder = next,
                None => return,
            }
        }
    }
    /// Reads `folder`, answers its loads, then the queued folders, until
    /// none is wanted.
    fn work(&self, mut folder: PathBuf) {
        loop {
            let mut slot = Slot {
                lister: self,
                folder: Some(folder.clone()),
                done: false,
            };
            let files = (self.read)(&folder);
            let waiters = self.lock().waiting.remove(&folder).unwrap_or_default();
            // Taken: from now on a panic only hands the place on (a newer
            // request for the same folder waits for a read of its own).
            slot.folder = None;
            for waiter in waiters {
                if waiter.alive.strong_count() > 0 {
                    (waiter.deliver)(&files);
                }
            }
            slot.done = true;
            match self.lock().hand_on() {
                Some(next) => folder = next,
                None => return,
            }
        }
    }
}
/// A reader's place while it reads one folder. Dropped without `done` (the
/// read or a ranking panicked): the folder's loads, if not taken yet, go
/// without, and the place passes to the next queued folder or is given back.
struct Slot<'a> {
    lister: &'a Lister,
    folder: Option<PathBuf>,
    done: bool,
}
impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let next = {
            let mut shared = self.lister.lock();
            if let Some(folder) = self.folder.take() {
                shared.waiting.remove(&folder);
            }
            shared.hand_on()
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
        if let State::Listing(Listing { receive, .. }) = &self.state {
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
    /// The folders read, in order.
    type Reads = Arc<Mutex<Vec<PathBuf>>>;
    fn read(reads: &Reads) -> Vec<PathBuf> {
        reads.lock().unwrap().clone()
    }
    /// A folder read that answers with the folder's cover.jpg, once released
    /// (folders named `quick…` at once).
    fn gated() -> (Lister, mpsc::Sender<()>, Reads) {
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let reads = Reads::default();
        let seen = reads.clone();
        let lister = Lister::new(move |folder| {
            seen.lock().unwrap().push(folder.to_path_buf());
            if !folder.to_string_lossy().starts_with("quick") {
                let _ = gate.lock().unwrap().recv();
            }
            vec![folder.join("cover.jpg")]
        });
        (lister, release, reads)
    }
    /// `MAX_RUNNING` folders being read and not answering.
    fn stuck(lister: &Lister, reads: &Reads) -> Vec<Covers> {
        let held = (0..MAX_RUNNING)
            .map(|n| lister.list(format!("stuck{n}").into(), None, names))
            .collect::<Vec<_>>();
        let end = Instant::now() + Duration::from_secs(5);
        while read(reads).len() < MAX_RUNNING {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
        held
    }
    fn answer(covers: Covers) -> Pictures {
        match covers {
            Covers::Now(found) => found,
            Covers::Later(Listing { receive, .. }) => receive
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
        while read(&reads).len() < MAX_RUNNING {
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
        assert_eq!(read(&reads).len(), MAX_RUNNING + 1);
        settled(&lister);
    }

    /// Quick skipping through a slow share: a queued folder whose load ended
    /// (its `Listing` dropped) is never read.
    #[test]
    fn a_queued_folder_whose_load_ended_is_not_read() {
        let (lister, release, reads) = gated();
        let held = stuck(&lister, &reads);
        drop(lister.list("quick-old".into(), None, names));
        let current = lister.list("quick-current".into(), None, names);
        release.send(()).unwrap();
        assert_eq!(
            answer(current),
            names(&[PathBuf::from("quick-current").join("cover.jpg")])
        );
        for _ in 1..MAX_RUNNING {
            release.send(()).unwrap();
        }
        drop(held);
        settled(&lister);
        assert!(
            !read(&reads).contains(&PathBuf::from("quick-old")),
            "{:?}",
            read(&reads)
        );
        assert!(lister.lock().waiting.is_empty());
    }

    /// Every reader stuck (the share stopped answering) and quick skipping
    /// through 50 folders: the withdrawn requests do not pile up.
    #[test]
    fn withdrawn_requests_do_not_pile_up_while_every_reader_is_stuck() {
        let (lister, release, reads) = gated();
        let held = stuck(&lister, &reads);
        for n in 0..50 {
            // Each load ends (its Listing dropped) before the next starts.
            drop(lister.list(format!("quick{n}").into(), None, names));
            drop(lister.list("quick-album".into(), None, names));
        }
        let current = lister.list("quick-current".into(), None, names);
        {
            let shared = lister.lock();
            assert_eq!(shared.queue, [PathBuf::from("quick-current")]);
            assert!(shared.waiting.values().all(|waiters| waiters.len() <= 1));
        }
        for _ in 0..MAX_RUNNING {
            release.send(()).unwrap();
        }
        assert_eq!(answer(current).len(), 1);
        drop(held);
        settled(&lister);
        let read = read(&reads);
        assert!(read.contains(&PathBuf::from("quick-current")));
        assert_eq!(read.len(), MAX_RUNNING + 1, "{read:?}");
        assert!(lister.lock().waiting.is_empty());
    }

    /// The file playing now is read before older requests still queued.
    #[test]
    fn the_current_folder_is_not_held_up_by_older_requests() {
        let (lister, release, reads) = gated();
        let held = stuck(&lister, &reads);
        let older = (0..3)
            .map(|n| lister.list(format!("quick-older{n}").into(), None, names))
            .collect::<Vec<_>>();
        let current = lister.list("quick-current".into(), None, names);
        release.send(()).unwrap();
        assert_eq!(answer(current).len(), 1);
        assert_eq!(
            read(&reads)[MAX_RUNNING],
            PathBuf::from("quick-current"),
            "the first folder read after the stuck ones"
        );
        // The older ones are still read (their loads are alive), newest first.
        for covers in older {
            assert_eq!(answer(covers).len(), 1);
        }
        assert_eq!(
            read(&reads)[MAX_RUNNING + 1..],
            ["quick-older2", "quick-older1", "quick-older0"].map(PathBuf::from)
        );
        for _ in 1..MAX_RUNNING {
            release.send(()).unwrap();
        }
        drop(held);
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
        assert_eq!(read(&reads).len(), 1);
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
        let Covers::Later(Listing { receive: bad, .. }) = lister.list("bad".into(), None, names)
        else {
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
        let Covers::Later(Listing { receive: bad, .. }) = bad else {
            panic!("late");
        };
        assert!(bad.recv_timeout(Duration::from_secs(5)).is_err());
        for covers in held {
            assert_eq!(answer(covers).len(), 1);
        }
        settled(&lister);
    }

    /// A ranking that panics after the folder's loads were taken: only the
    /// place is handed on; a later request for that folder gets its own read.
    #[test]
    fn a_ranking_that_panics_hands_on_only_the_place() {
        let lister = Lister::new(|folder| vec![folder.join("cover.jpg")]);
        let Covers::Later(Listing { receive: bad, .. }) =
            lister.list("x".into(), None, |_| panic!("ranking"))
        else {
            panic!("late");
        };
        assert!(bad.recv_timeout(Duration::from_secs(5)).is_err());
        settled(&lister);
        assert!(lister.lock().waiting.is_empty());
        let again = lister.list("x".into(), None, names);
        assert_eq!(
            answer(again),
            names(&[PathBuf::from("x").join("cover.jpg")])
        );
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
