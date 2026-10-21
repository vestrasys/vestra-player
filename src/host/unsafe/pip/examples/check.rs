//! This crate's own PiP check: a standalone program (no Vestra). It opens a
//! window whose content is covered by an opaque view, places
//! [`vestra_pip::Pip`] on a "video area" of it, feeds RGB frames of seven
//! vertical bars, starts PiP, grows the window while PiP is open, stops it
//! and exits. It prints every event and, while PiP is open, the geometry
//! line (`Pip::describe`): the layer must have the PiP window's size and the
//! render size AVKit reports must match it.
//!
//! Default: a 640×360 window, video area 520×312 in its top-left corner (the
//! rest stands for the side bar and bottom bar), 1920×1080 frames.
//!
//! Like the host (`--frame WxH --area WxH`): the window's content is the
//! video area and frames are `--frame` pixels. While PiP wants frames the
//! host renders at the picture's own shape inside the video area (no bars),
//! so for a 16:9 picture in a 780×671 area on a Retina screen pass
//! `--frame 1560x878 --area 780x671`. `--still`: one frame only (paused
//! playback). The PiP window should show all seven bars, filling it, with
//! no black or white bands.
//!
//! `cargo run -p vestra-pip --example check [-- --frame 1560x878 --area 780x671]`
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSEventMask, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
    use std::time::{Duration, Instant};

    let args: Vec<String> = std::env::args().collect();
    let size_arg = |name: &str| {
        let at = args.iter().position(|a| a == name)?;
        let (w, h) = args.get(at + 1)?.split_once('x')?;
        Some((w.parse::<usize>().ok()?, h.parse::<usize>().ok()?))
    };
    let host_area = size_arg("--area");
    let (width, height) = size_arg("--frame").unwrap_or((1920, 1080));
    let still = args.iter().any(|a| a == "--still");

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.finishLaunching();
    let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let (w, h) = host_area.map_or((640., 360.), |(w, h)| (w as f64, h as f64));
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    // SAFETY: NSWindow's designated initializer on a fresh allocation, main
    // thread; not released when closed (owned by `Retained`).
    let window = unsafe {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(200., 200., w, h),
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window
    };
    window.setTitle(&objc2_foundation::NSString::from_str("vestra-pip check"));
    let content = window.contentView().expect("content view");
    let cover = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
    cover.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    cover.setWantsLayer(true);
    if let Some(layer) = cover.layer() {
        layer.setBackgroundColor(Some(&objc2_core_graphics::CGColor::new_generic_rgb(
            1., 0., 0., 1.,
        )));
    }
    content.addSubview(&cover);
    window.makeKeyAndOrderFront(None);
    app.activate();

    let pip = vestra_pip::Pip::new().expect("PiP supported");
    let area = |width: f64, height: f64| match host_area {
        Some(_) => rect(0., 0., width, height),
        None => rect(0., 0., width - 120., height - 48.),
    };
    let mut frame = vec![0u8; width * height * 3];
    let started = Instant::now();
    let (mut asked, mut grown, mut shown, mut stopped) = (false, false, false, false);
    let mut n = 0usize;
    while started.elapsed() < Duration::from_secs(9) {
        // SAFETY: the shared application's event loop, on the main thread.
        let event = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::dateWithTimeIntervalSinceNow(1. / 30.)),
                NSDefaultRunLoopMode,
                true,
            )
        };
        if let Some(event) = event {
            app.sendEvent(&event);
        }
        let size = content.bounds().size;
        assert!(pip.place(&window, area(size.width, size.height)));
        if !still || n == 0 {
            for (i, pixel) in frame.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let bar = ((i % width + n * 8) % width) * 7 / width;
                *pixel = [[255, 0, 0], [0, 255, 0], [0, 0, 255]][bar % 3];
            }
            pip.enqueue(width, height, &frame).expect("frame");
        }
        n += 1;
        pip.set_playback(still, Some(n as f64 / 30.), Some(600.), 1.);
        let t = started.elapsed().as_secs_f64();
        if !asked && t > 1.5 {
            asked = pip.start();
            println!("start asked {asked} (possible {})", pip.possible());
        }
        if !shown && t > 3.5 {
            shown = true;
            println!("{t:.1}s {}", pip.describe(&window));
        }
        if !grown && t > 4.5 {
            grown = true;
            window.setContentSize(NSSize::new(size.width * 1.25, size.height * 1.25));
            println!("grown to {}x{}", size.width * 1.25, size.height * 1.25);
        }
        if !stopped && t > 7. {
            stopped = true;
            println!("{t:.1}s {}", pip.describe(&window));
            pip.stop();
        }
        for event in pip.events() {
            println!("{t:.1}s event {event:?} renderSize {:?}", pip.render_size());
        }
    }
    println!("{}", pip.describe(&window));
}

#[cfg(not(target_os = "macos"))]
fn main() {}
