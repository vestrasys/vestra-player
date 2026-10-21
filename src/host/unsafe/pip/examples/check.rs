//! This crate's own PiP check: a standalone program (no Vestra) that opens
//! one 640×360 window whose content is covered by an opaque view, places
//! [`vestra_pip::Pip`] on a 520×312 "video area" in its top-left corner (the
//! rest stands for the side bar and bottom bar), feeds 1920×1080 RGB frames,
//! starts PiP, grows the window while PiP is open, stops it and exits. It
//! prints the render size AVKit reports (it should equal the video area's
//! 16:9 picture, 520×292, not something small) and every event.
//!
//! `cargo run -p vestra-pip --example check`
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType};
    use objc2_app_kit::{NSEventMask, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
    use std::time::{Duration, Instant};

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.finishLaunching();
    let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    // SAFETY: NSWindow's designated initializer on a fresh allocation, main
    // thread; not released when closed (owned by `Retained`).
    let window = unsafe {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(200., 200., 640., 360.),
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
    let area = |width: f64, height: f64| rect(0., 0., width - 120., height - 48.);
    let (width, height) = (1920, 1080);
    let mut frame = vec![0u8; width * height * 3];
    let started = Instant::now();
    let (mut asked, mut grown, mut stopped) = (false, false, false);
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
        for (i, pixel) in frame.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let bar = ((i % width + n * 8) % width) * 7 / width;
            *pixel = [[255, 0, 0], [0, 255, 0], [0, 0, 255]][bar % 3];
        }
        n += 1;
        pip.enqueue(width, height, &frame).expect("frame");
        pip.set_playback(false, Some(n as f64 / 30.), Some(600.), 1.);
        let t = started.elapsed().as_secs_f64();
        if !asked && t > 1.5 {
            asked = pip.start();
            println!("start asked {asked} (possible {})", pip.possible());
        }
        if !grown && t > 4.5 {
            grown = true;
            window.setContentSize(NSSize::new(960., 540.));
            println!("grown to 960x540 (video area 840x492)");
        }
        if !stopped && t > 7. {
            stopped = true;
            pip.stop();
        }
        for event in pip.events() {
            println!(
                "{t:.1}s event {event:?} renderSize {:?} active {}",
                pip.render_size(),
                pip.active()
            );
        }
    }
    println!("renderSize {:?}", pip.render_size());
}

#[cfg(not(target_os = "macos"))]
fn main() {}
