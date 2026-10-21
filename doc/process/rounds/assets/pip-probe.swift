import AppKit
import AVKit
import AVFoundation
import CoreMedia
import CoreVideo

// Standalone PiP probe: no Vestra. Variants via argv[1]:
//  normal  - display layer fills the window's content view (Apple's recommended setup)
//  small   - display layer is a 160x90 sublayer (like Vestra's prototype)
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
    return sb!
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
let win = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 640, height: 360), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
win.title = "pip probe \(variant)"
let view = win.contentView!
view.wantsLayer = true
let layer = AVSampleBufferDisplayLayer()
layer.videoGravity = .resizeAspect
layer.backgroundColor = NSColor.black.cgColor
if variant == "small" {
    layer.frame = CGRect(x: 0, y: 0, width: 160, height: 90)
} else {
    layer.frame = view.bounds
    layer.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
}
view.layer!.addSublayer(layer)
var tb: CMTimebase?
CMTimebaseCreateWithSourceClock(allocator: nil, sourceClock: CMClockGetHostTimeClock(), timebaseOut: &tb)
CMTimebaseSetTime(tb!, time: .zero); CMTimebaseSetRate(tb!, rate: 1)
layer.controlTimebase = tb
win.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)

let del = Delegate()
let src = AVPictureInPictureController.ContentSource(sampleBufferDisplayLayer: layer, playbackDelegate: del)
let pip = AVPictureInPictureController(contentSource: src)
pip.delegate = del
var n = 0
Timer.scheduledTimer(withTimeInterval: 1.0/30, repeats: true) { _ in
    if layer.isReadyForMoreMediaData { layer.enqueue(frame(n)); n += 1 }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
    print("possible \(pip.isPictureInPicturePossible)")
    pip.startPictureInPicture()
}
DispatchQueue.main.asyncAfter(deadline: .now() + 9) { app.terminate(nil) }
app.run()
