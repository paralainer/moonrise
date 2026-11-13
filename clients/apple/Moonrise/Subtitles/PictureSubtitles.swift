import CoreGraphics
import Foundation
import ImageIO

/// One cue of a picture subtitle track (PGS, DVD, DVB): an image the server decoded from the
/// track, placed on the track's canvas (the video frame it was made for), shown from `start` to
/// `end` on the title's timeline. Two cues are the same one when their time and place are (a cue
/// showing across a window boundary comes in both windows).
struct PictureCue: Hashable {
    let start: Double
    let end: Double
    /// Its place on the canvas.
    let rect: CGRect
    /// Decoded once, when the cue arrives.
    let image: CGImage?

    init(start: Double, end: Double, rect: CGRect, image: CGImage?) {
        self.start = start
        self.end = end
        self.rect = rect
        self.image = image
    }

    /// A cue of a window (`MoonriseServer.SubtitleWindow`): its PNG decoded. Nil without an image.
    init?(_ c: MoonriseServer.SubtitleWindow.Cue) {
        guard let i = c.image else { return nil }
        self.init(start: c.start, end: c.end, rect: CGRect(x: i.x, y: i.y, width: i.width, height: i.height),
                  image: Data(base64Encoded: i.png).flatMap(Self.decode))
    }

    static func decode(_ png: Data) -> CGImage? {
        guard let src = CGImageSourceCreateWithData(png as CFData, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(src, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
    }

    static func == (a: PictureCue, b: PictureCue) -> Bool {
        a.start == b.start && a.end == b.end && a.rect.origin == b.rect.origin
    }

    func hash(into h: inout Hasher) {
        h.combine(start)
        h.combine(end)
        h.combine(rect.origin.x)
        h.combine(rect.origin.y)
    }
}

/// The picture cues known so far of the selected track, sorted by start, and their canvas.
struct PictureDocument {
    let canvas: CGSize
    let cues: [PictureCue]

    init(canvas: CGSize, cues: [PictureCue]) {
        self.canvas = canvas
        self.cues = cues.sorted { ($0.start, $0.end, $0.rect.minY) < ($1.start, $1.end, $1.rect.minY) }
    }

    /// The cues showing at `t` (several at once: two lines placed apart, a sign and dialogue).
    func showing(at t: Double) -> [PictureCue] {
        var lo = 0, hi = cues.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if cues[mid].start <= t { lo = mid + 1 } else { hi = mid }
        }
        // Cues starting at or before t; long ones may have started a while back.
        return cues[max(0, lo - 16)..<lo].filter { t < $0.end }
    }

    /// The next time after `t` where what shows changes (a cue starts or ends).
    func nextEdge(after t: Double) -> Double? {
        var lo = 0, hi = cues.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if cues[mid].start <= t { lo = mid + 1 } else { hi = mid }
        }
        var edge = lo < cues.count ? cues[lo].start : nil
        for c in cues[max(0, lo - 16)..<lo] where c.end > t { edge = min(edge ?? c.end, c.end) }
        return edge
    }
}

/// Where picture cues are drawn: the canvas is the video's frame, wherever the video shows.
enum PictureLayout {
    /// The rectangle the video is drawn in inside `bounds`: its picture size (`video`, the
    /// display size, so an anamorphic DVD is already as wide as it shows) fitted in, or filling
    /// it (`fill`, cropped). Without a size yet: the canvas's shape.
    static func videoRect(video: CGSize, canvas: CGSize, in bounds: CGSize, fill: Bool) -> CGRect {
        let shape = video.width > 0 && video.height > 0 ? video : canvas
        guard shape.width > 0, shape.height > 0, bounds.width > 0, bounds.height > 0 else {
            return CGRect(origin: .zero, size: bounds)
        }
        let sx = bounds.width / shape.width, sy = bounds.height / shape.height
        let s = fill ? max(sx, sy) : min(sx, sy)
        let w = shape.width * s, h = shape.height * s
        return CGRect(x: (bounds.width - w) / 2, y: (bounds.height - h) / 2, width: w, height: h)
    }

    /// A cue's frame in the view: the canvas stretched over `videoRect`, across and down on
    /// their own (a 720×480 DVD canvas covers a 16:9 picture).
    static func frame(_ rect: CGRect, canvas: CGSize, videoRect: CGRect) -> CGRect {
        guard canvas.width > 0, canvas.height > 0 else { return .zero }
        let sx = videoRect.width / canvas.width, sy = videoRect.height / canvas.height
        return CGRect(x: videoRect.minX + rect.minX * sx, y: videoRect.minY + rect.minY * sy,
                      width: rect.width * sx, height: rect.height * sy)
    }

    /// How far up the cues in the lower half of the `stage` go, as one block, to stay clear of
    /// what covers the bottom of the picture (`frames` and `stage` in the same space): Apple TV's
    /// transport bar (`raisedBottom`, from the stage's bottom: they end above it), or our controls
    /// (`controls`: only when a control would cover one, they end above the bottom block, as
    /// text does). 0 when nothing is in the way; never past the stage's top.
    static func lift(_ frames: [CGRect], stage: CGRect, controls: ControlsLayout? = nil, raisedBottom: CGFloat? = nil) -> CGFloat {
        let low = frames.filter { $0.midY > stage.midY }
        guard let bottom = low.map(\.maxY).max(), let top = low.map(\.minY).min() else { return 0 }
        let limit: CGFloat
        if let raisedBottom {
            limit = stage.maxY - raisedBottom
            guard bottom > limit else { return 0 }
        } else if let controls {
            let bandTop = controls.parts.filter { $0.kind == .bottom }.map(\.frame.minY).min() ?? stage.maxY
            let covered = low.contains { f in f.maxY > bandTop || controls.parts.contains { $0.kind != .bottom && $0.hides(f) } }
            guard covered else { return 0 }
            limit = bandTop - 8
        } else {
            return 0
        }
        return max(0, min(bottom - limit, top - stage.minY))
    }
}
