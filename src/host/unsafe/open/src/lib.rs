//! Files the system asks the player to open on macOS: Finder's "Open With"
//! and double-click, a drop on the Dock icon, `open -a`. LaunchServices
//! sends them to the running app as an open-documents Apple event (`aevt`
//! `odoc`), at launch as well (they are not on the command line then).
//!
//! winit (through Slint) owns the application delegate and implements no
//! `application:openURLs:`, so AppKit's default handler would answer with
//! "cannot open files in the … format" (it looks for an NSDocument class).
//! This crate installs its own handler with the Apple event manager instead:
//! right away, and again when the application will finish launching, which
//! is where AppKit installs its standard handlers and before it dispatches
//! the launch's own open event.
//!
//! Main thread only. All `unsafe` stays in this crate; the public API is safe.
#![cfg(target_os = "macos")]

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString,
};
use std::cell::RefCell;
use std::path::PathBuf;

/// FourCharCodes from the Apple Event Manager (AE/AppleEvents.h, AEDataModel.h).
const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const K_AE_OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

pub struct State {
    deliver: Box<dyn Fn(Vec<PathBuf>)>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the class adds only
    // its Rust ivars and is used on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VestraOpenFiles"]
    #[ivars = State]
    struct Handler;

    // SAFETY: NSObjectProtocol has no required methods beyond NSObject's.
    unsafe impl NSObjectProtocol for Handler {}

    impl Handler {
        // SAFETY: the signature NSAppleEventManager calls handlers with
        // (`- (void)handleEvent:(NSAppleEventDescriptor *)event
        // withReplyEvent:(NSAppleEventDescriptor *)reply`).
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let files = files(event);
            if !files.is_empty() {
                (self.ivars().deliver)(files);
            }
        }

        // SAFETY: a notification observer's selector takes the notification.
        #[unsafe(method(applicationWillFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            self.install();
        }
    }
);

impl Handler {
    fn install(&self) {
        let manager = NSAppleEventManager::sharedAppleEventManager();
        // SAFETY: `self` responds to the selector with the handler signature
        // (above) and stays alive for the rest of the process (`HANDLER`);
        // the class and ID are AEEventClass/AEEventID (UInt32).
        unsafe {
            let _: () = msg_send![
                &*manager,
                setEventHandler: self,
                andSelector: sel!(handleOpenDocuments:withReplyEvent:),
                forEventClass: K_CORE_EVENT_CLASS,
                andEventID: K_AE_OPEN_DOCUMENTS,
            ];
        }
    }
}

thread_local! {
    /// The Apple event manager and the notification center do not retain
    /// their handler and observer.
    static HANDLER: RefCell<Option<Retained<Handler>>> = const { RefCell::new(None) };
}

/// The paths of an open-documents event's direct object: a list of file
/// references (or a single one).
fn files(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    // SAFETY: `paramDescriptorForKeyword:` takes an AEKeyword (UInt32) and
    // returns a descriptor or nil.
    let list: Option<Retained<NSAppleEventDescriptor>> =
        unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
    let Some(list) = list else { return Vec::new() };
    let path = |item: &NSAppleEventDescriptor| {
        item.fileURLValue().and_then(|url| url.path()).map(|path| PathBuf::from(path.to_string()))
    };
    let count = list.numberOfItems();
    if count <= 0 {
        return path(&list).into_iter().collect();
    }
    (1..=count)
        .filter_map(|index| list.descriptorAtIndex(index))
        .filter_map(|item| path(&item))
        .collect()
}

/// Send the files of every open-documents event to `deliver`, from now on
/// (the launch's own event included when this runs before the event loop).
/// On the main thread, once: `false` elsewhere and on a second call.
pub fn on_open_files(deliver: impl Fn(Vec<PathBuf>) + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    if HANDLER.with(|slot| slot.borrow().is_some()) {
        return false;
    }
    let handler = Handler::alloc(mtm).set_ivars(State { deliver: Box::new(deliver) });
    // SAFETY: NSObject's designated initializer on a freshly allocated object.
    let handler: Retained<Handler> = unsafe { msg_send![super(handler), init] };
    handler.install();
    let center = NSNotificationCenter::defaultCenter();
    let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
    // SAFETY: `handler` responds to the selector with a notification
    // argument and stays alive for the rest of the process (`HANDLER`).
    unsafe {
        center.addObserver_selector_name_object(
            &handler,
            sel!(applicationWillFinishLaunching:),
            Some(&name),
            None,
        );
    }
    HANDLER.with(|slot| *slot.borrow_mut() = Some(handler));
    true
}
