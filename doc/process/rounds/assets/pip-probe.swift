import AppKit
import AVKit
import AVFoundation
import CoreMedia
import CoreVideo
import OpenGL.GL

// Standalone PiP probe: no Vestra. Variants via argv[1]:
//  normal   - display layer fills the window's content view (Apple's recommended setup)
//  small    - display layer is a 160x90 sublayer (like Vestra's prototype)
//  covered  - as normal, but an opaque view sits on top of the layer (layer hidden from sight)
//  area     - layer = a "video area" inside the content view (bottom bar 48 pt, side bar 120 pt)
//  gl       - like Vestra: a flipped, layer-backed content view drawn by an NSOpenGLContext
//             (opaque red); the layer is inserted at index 0 of the view's layer, on the
//             video area, under the GL surface
//  glresize - as gl, the window grows to 960x540 before PiP starts; the host moves the layer
//  offscreen- layer the size of the window, but in a view left of the window (x < 0)
//  hidden   - as normal, layer.isHidden = true
//  clear    - as normal, layer.opacity = 0
//  child    - layer fills a borderless child window ordered below the main window, same frame
//             as the main window's content (fully covered by the opaque main window)
//  childgl  - Vestra's formal setup: gl main window; the layer fills a child window ordered
//             below it whose frame is the main window's video area (screen coordinates); the
//             window grows to 960x540 while PiP is open and the child follows
//  grow     - as normal, the window grows to 960x540 while PiP is open
//  bigframe - as normal, but 1920x1080 frames marked "display immediately" (Vestra's feed)
// Logs: renderSize (AVKit's render size: the layer's on-screen video rectangle; a small one is
// the white-edge case), the layer's frame and superlayer before/while PiP is open (AVKit does
// not move or resize the source layer), and for gl the sublayer order (a sublayer of a GL
// view's backing layer draws above the GL contents, so it would cover the player's overlays).
// Results 2026-10-21 (round render-pip §3.1): normal/covered/child/offscreen/clear/bigframe/grow
// 640x360, small
// 160x90, area/gl/childgl 520x292, glresize 840x472, hidden none (no renderSize).
let variant = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "normal"
let W = 640, H = 360

final class Delegate: NSObject, AVPictureInPictureSampleBufferPlaybackDelegate, AVPictureInPictureControllerDelegate {
    func pictureInPictureController(_ c: AVPictureInPictureController, setPlaying playing: Bool) {}
    func pictureInPictureControllerTimeRangeForPlayback(_ c: AVPictureInPictureController) -> CMTimeRange {
        CMTimeRange(start: .negativeInfinity, duration: .positiveInfinity)
    }
    func pictureInPictureControllerIsPlaybackPaused(_ c: AVPictureInPictureController) -> Bool { false }
    func pictureInPictureController(_ c: AVPictureInPictureController, didTransitionToRenderSize s: CMVideoDimensions) {
        print("renderSize \(s.width)x\(s.height)")
    }
    func pictureInPictureController(_ c: AVPictureInPictureController, skipByInterval i: CMTime, completion: @escaping () -> Void) { completion() }
    func pictureInPictureControllerDidStartPictureInPicture(_ c: AVPictureInPictureController) { print("didStart") }
    func pictureInPictureController(_ c: AVPictureInPictureController, failedToStartPictureInPictureWithError e: Error) { print("failed \(e)") }
}

func frame(_ n: Int) -> CMSampleBuffer {
    let (W, H) = variant == "bigframe" ? (1920, 1080) : (W, H)
    var pb: CVPixelBuffer?
    CVPixelBufferCreate(nil, W, H, kCVPixelFormatType_32BGRA, [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &pb)
    let p = pb!
    CVPixelBufferLockBaseAddress(p, [])
    let base = CVPixelBufferGetBaseAddress(p)!.assumingMemoryBound(to: UInt8.self)
    let stride = CVPixelBufferGetBytesPerRow(p)
    let colors: [(UInt8, UInt8, UInt8)] = [(0,0,255),(0,255,0),(255,0,0),(0,255,255),(255,0,255),(255,255,0),(255,255,255)]
    for y in 0..<H { for x in 0..<W {
        let c = colors[min(6, (x + n * 4) % W * 7 / W)]
        let o = y * stride + x * 4
        base[o] = c.0; base[o+1] = c.1; base[o+2] = c.2; base[o+3] = 255
    } }
    CVPixelBufferUnlockBaseAddress(p, [])
    var fmt: CMVideoFormatDescription?
    CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: p, formatDescriptionOut: &fmt)
    var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 30), presentationTimeStamp: CMTime(value: CMTimeValue(n), timescale: 30), decodeTimeStamp: .invalid)
    var sb: CMSampleBuffer?
    CMSampleBufferCreateReadyWithImageBuffer(allocator: nil, imageBuffer: p, formatDescription: fmt!, sampleTiming: &timing, sampleBufferOut: &sb)
    if variant == "bigframe", let a = CMSampleBufferGetSampleAttachmentsArray(sb!, createIfNecessary: true) {
        let d = unsafeBitCast(CFArrayGetValueAtIndex(a, 0), to: CFMutableDictionary.self)
        CFDictionarySetValue(d, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(), Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
    }
    return sb!
}

/// Winit's content view: flipped (origin top-left), layer-backed before GL attaches.
final class FlippedView: NSView { override var isFlipped: Bool { true } }

/// The "video area" of a view this size: a 120 pt side bar on the right, a 48 pt bar at the bottom.
func videoArea(_ b: NSRect, flipped: Bool) -> NSRect {
    let h = b.height - 48
    return NSRect(x: 0, y: flipped ? 0 : 48, width: b.width - 120, height: h)
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let win = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 640, height: 360), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
win.title = "pip probe \(variant)"
let gl = variant.hasPrefix("gl") || variant == "childgl"
if gl {
    let v = FlippedView(frame: NSRect(x: 0, y: 0, width: 640, height: 360))
    v.wantsLayer = true
    win.contentView = v
}
let view = win.contentView!
view.wantsLayer = true
let layer = AVSampleBufferDisplayLayer()
layer.videoGravity = .resizeAspect
layer.backgroundColor = NSColor.black.cgColor
var glContext: NSOpenGLContext?
var childWindow: NSWindow?
/// Put the child window on the main window's video area and the layer on all of it.
func placeChild() {
    let area = videoArea(view.bounds, flipped: true)
    let inWindow = view.convert(area, to: nil)
    childWindow!.setFrame(win.convertToScreen(inWindow), display: false)
    CATransaction.begin(); CATransaction.setDisableActions(true)
    layer.frame = childWindow!.contentView!.bounds
    CATransaction.commit()
}
switch variant {
case "childgl":
    let attrs: [NSOpenGLPixelFormatAttribute] = [UInt32(NSOpenGLPFADoubleBuffer), UInt32(NSOpenGLPFAOpenGLProfile), UInt32(NSOpenGLProfileVersion3_2Core), 0]
    let ctx = NSOpenGLContext(format: NSOpenGLPixelFormat(attributes: attrs)!, share: nil)!
    ctx.view = view
    glContext = ctx
    let child = NSWindow(contentRect: .zero, styleMask: [.borderless], backing: .buffered, defer: false)
    child.ignoresMouseEvents = true
    child.hasShadow = false
    child.collectionBehavior = [.fullScreenAuxiliary, .ignoresCycle]
    child.contentView!.wantsLayer = true
    child.contentView!.layer!.addSublayer(layer)
    childWindow = child
    placeChild()
    win.addChildWindow(child, ordered: .below)
case "small":
    layer.frame = CGRect(x: 0, y: 0, width: 160, height: 90)
    view.layer!.addSublayer(layer)
case "area":
    layer.frame = videoArea(view.bounds, flipped: false)
    layer.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
    view.layer!.addSublayer(layer)
case "gl", "glresize":
    let attrs: [NSOpenGLPixelFormatAttribute] = [UInt32(NSOpenGLPFADoubleBuffer), UInt32(NSOpenGLPFAOpenGLProfile), UInt32(NSOpenGLProfileVersion3_2Core), 0]
    let ctx = NSOpenGLContext(format: NSOpenGLPixelFormat(attributes: attrs)!, share: nil)!
    ctx.view = view
    glContext = ctx
    layer.frame = videoArea(view.bounds, flipped: true)
    view.layer!.insertSublayer(layer, at: 0)
case "offscreen":
    let host = NSView(frame: NSRect(x: -700, y: 0, width: 640, height: 360))
    host.wantsLayer = true
    view.addSubview(host)
    layer.frame = host.bounds
    layer.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
    host.layer!.addSublayer(layer)
case "child":
    let child = NSWindow(contentRect: win.contentRect(forFrameRect: win.frame), styleMask: [.borderless], backing: .buffered, defer: false)
    child.contentView!.wantsLayer = true
    layer.frame = child.contentView!.bounds
    layer.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
    child.contentView!.layer!.addSublayer(layer)
    win.addChildWindow(child, ordered: .below)
    let cover = NSView(frame: view.bounds)
    cover.wantsLayer = true
    cover.layer!.backgroundColor = NSColor.red.cgColor
    cover.autoresizingMask = [.width, .height]
    view.addSubview(cover)
default: // normal, covered, hidden, clear, bigframe
    layer.frame = view.bounds
    layer.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
    view.layer!.addSublayer(layer)
}
if variant == "hidden" { layer.isHidden = true }
if variant == "clear" { layer.opacity = 0 }
if variant == "covered" {
    let cover = NSView(frame: view.bounds)
    cover.wantsLayer = true
    cover.layer!.backgroundColor = NSColor.red.cgColor
    cover.autoresizingMask = [.width, .height]
    view.addSubview(cover)
}
var tb: CMTimebase?
CMTimebaseCreateWithSourceClock(allocator: nil, sourceClock: CMClockGetHostTimeClock(), timebaseOut: &tb)
CMTimebaseSetTime(tb!, time: .zero); CMTimebaseSetRate(tb!, rate: 1)
layer.controlTimebase = tb
win.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)

func describe(_ l: CALayer?) -> String {
    guard let l else { return "nil" }
    let f = l.frame
    return "\(type(of: l)) \(Int(f.origin.x)),\(Int(f.origin.y)) \(Int(f.width))x\(Int(f.height))"
}
func report(_ tag: String) {
    let parent = layer.superlayer
    let fills = parent.map { $0.bounds.size == layer.frame.size } ?? false
    print("\(tag): active \(pip.isPictureInPictureActive) layer \(describe(layer)) in \(describe(parent)) fills \(fills)")
}

let del = Delegate()
let src = AVPictureInPictureController.ContentSource(sampleBufferDisplayLayer: layer, playbackDelegate: del)
let pip = AVPictureInPictureController(contentSource: src)
pip.delegate = del
var n = 0
Timer.scheduledTimer(withTimeInterval: 1.0/30, repeats: true) { _ in
    if let ctx = glContext {
        ctx.makeCurrentContext()
        ctx.update()
        glClearColor(1, 0, 0, 1)
        glClear(GLbitfield(GL_COLOR_BUFFER_BIT))
        ctx.flushBuffer()
    }
    if layer.isReadyForMoreMediaData { layer.enqueue(frame(n)); n += 1 }
}
if variant == "glresize" {
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.7) {
        win.setContentSize(NSSize(width: 960, height: 540))
        // The host re-places the layer on the video area (no autoresizing in a GL view).
        CATransaction.begin(); CATransaction.setDisableActions(true)
        layer.frame = videoArea(view.bounds, flipped: true)
        CATransaction.commit()
    }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 1.3) {
    if gl {
        let order = (view.layer!.sublayers ?? []).map { describe($0) }.joined(separator: " | ")
        print("view layer \(describe(view.layer)) contents \(view.layer!.contents != nil) sublayers: \(order)")
    }
    report("before")
}
DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
    print("possible \(pip.isPictureInPicturePossible)")
    pip.startPictureInPicture()
}
DispatchQueue.main.asyncAfter(deadline: .now() + 4) {
    report("during")
    if variant == "grow" || variant == "childgl" {
        win.setContentSize(NSSize(width: 960, height: 540))
        if childWindow != nil { placeChild() }
        let child = childWindow.map { "\($0.frame) occlusion visible \($0.occlusionState.contains(.visible))" } ?? "-"
        print("grown; child \(child)")
    }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 7) { report("later") }
DispatchQueue.main.asyncAfter(deadline: .now() + 9) { app.terminate(nil) }
app.run()
