// Picture subtitles (PGS, DVD, DVB) as the server sends them and the overlay draws them:
// Moonrise/Subtitles/PictureSubtitles.swift (the window's images, which cues show when, where
// on the video they go) and MoonriseServer.SubtitleWindow.
import CoreGraphics
import Foundation
import Testing
@testable import Moonrise

@Suite struct PictureSubtitlesTests {
    /// A 2×2 white PNG.
    private static let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAADklEQVR4nGP4DwUMMAYAj4IP8TylVlEAAAAASUVORK5CYII="

    private static let window = """
    {"index": 2, "window": 1, "from": 30.0, "to": 60.0, "span": 30.0, "complete": false, "source": "index",
     "canvas": {"width": 1920, "height": 1080},
     "cues": [{"start": 31.5, "end": 34.0, "text": "", "image": {"x": 777, "y": 949, "width": 354, "height": 48, "png": "\(png)"}},
              {"start": 35.0, "end": 38.0, "text": "", "image": {"x": 777, "y": 63, "width": 360, "height": 48, "png": "\(png)"}}]}
    """

    private func cue(_ start: Double, _ end: Double, x: Double = 0, y: Double = 0) -> PictureCue {
        PictureCue(start: start, end: end, rect: CGRect(x: x, y: y, width: 10, height: 10), image: nil)
    }

    /// A picture track's window: its canvas, and each cue's place and image (decoded once).
    @Test func windowWithImagesDecodes() throws {
        let w = try MoonriseServer.snake.decode(MoonriseServer.SubtitleWindow.self, from: Data(Self.window.utf8))
        #expect(w.window == 1 && !w.complete)
        #expect(w.canvas?.width == 1920 && w.canvas?.height == 1080)
        let cues = w.cues.compactMap(PictureCue.init)
        #expect(cues.count == 2)
        #expect(cues[0].rect == CGRect(x: 777, y: 949, width: 354, height: 48))
        #expect(cues[0].image?.width == 2 && cues[0].image?.height == 2)
        // A text track's window has neither.
        let text = try MoonriseServer.snake.decode(MoonriseServer.SubtitleWindow.self, from: Data("""
        {"index": 3, "window": 0, "from": 0, "to": 30, "span": 30, "complete": true, "source": "file",
         "cues": [{"start": 1, "end": 4, "text": "Hello, world."}]}
        """.utf8))
        #expect(text.canvas == nil && text.cues[0].image == nil)
        #expect(PictureCue(text.cues[0]) == nil)
    }

    /// What shows at a time: the cues covering it (several at once), none in a gap; the next
    /// change after it; a cue that came in two windows counts once.
    @Test func cuesShowingAndEdges() {
        let doc = PictureDocument(canvas: CGSize(width: 1920, height: 1080), cues: [
            cue(10, 13, y: 900), cue(0, 3), cue(11, 12, y: 60), cue(20, 23),
        ])
        #expect(doc.showing(at: 1).count == 1)
        #expect(doc.showing(at: 3).isEmpty, "the end is exclusive")
        #expect(doc.showing(at: 5).isEmpty)
        #expect(Set(doc.showing(at: 11.5).map(\.rect.minY)) == [900, 60], "two at once")
        #expect(doc.nextEdge(after: 5) == 10)
        #expect(doc.nextEdge(after: 10.5) == 11)
        #expect(doc.nextEdge(after: 11.5) == 12)
        #expect(doc.nextEdge(after: 30) == nil)
        #expect(Set([cue(1, 2, x: 5), cue(1, 2, x: 5)]).count == 1)
        #expect(Set([cue(1, 2, x: 5), cue(1, 2, x: 6)]).count == 2)
    }

    /// The subtitle delay applies as to text: the player asks for `t - delay`.
    @Test func delayMovesPictures() {
        let doc = PictureDocument(canvas: CGSize(width: 720, height: 576), cues: [cue(10, 12)])
        let delay = 1.5
        #expect(doc.showing(at: 11 - delay).isEmpty, "shown later by the delay")
        #expect(doc.showing(at: 12 - delay).count == 1)
        #expect(doc.showing(at: 13.4 - delay).count == 1)
        #expect(doc.showing(at: 13.6 - delay).isEmpty)
    }

    /// The canvas goes over the rectangle the video is drawn in: letterboxed, pillarboxed,
    /// filling (cropped), and a 4:3-pixel DVD canvas over a 16:9 picture.
    @Test func canvasOverTheVideo() {
        let hd = CGSize(width: 1920, height: 1080)
        // A 16:9 video in a 4:3 view: bars above and below.
        let r = PictureLayout.videoRect(video: hd, canvas: hd, in: CGSize(width: 1024, height: 768), fill: false)
        #expect(r == CGRect(x: 0, y: 96, width: 1024, height: 576))
        let f = PictureLayout.frame(CGRect(x: 777, y: 949, width: 354, height: 48), canvas: hd, videoRect: r)
        #expect(abs(f.minX - 414.4) < 0.01 && abs(f.minY - (96 + 506.13)) < 0.01 && abs(f.width - 188.8) < 0.01)
        // A 16:9 video in a wider view: bars at the sides.
        let wide = PictureLayout.videoRect(video: hd, canvas: hd, in: CGSize(width: 2400, height: 1080), fill: false)
        #expect(wide == CGRect(x: 240, y: 0, width: 1920, height: 1080))
        // Filling a 4:3 view: wider than the view, cropped at the sides.
        let fill = PictureLayout.videoRect(video: hd, canvas: hd, in: CGSize(width: 1024, height: 768), fill: true)
        #expect(abs(fill.width - 1365.33) < 0.01 && fill.height == 768 && fill.minX < 0)
        // An anamorphic DVD: a 720×480 canvas over a picture that shows 16:9 (its display size).
        let dvd = PictureLayout.videoRect(video: CGSize(width: 864, height: 486), canvas: CGSize(width: 720, height: 480),
                                          in: CGSize(width: 1920, height: 1080), fill: false)
        #expect(dvd == CGRect(x: 0, y: 0, width: 1920, height: 1080))
        let line = PictureLayout.frame(CGRect(x: 180, y: 400, width: 360, height: 40), canvas: CGSize(width: 720, height: 480), videoRect: dvd)
        #expect(line == CGRect(x: 480, y: 900, width: 960, height: 90), "across and down on their own")
        // Before the video's size is known: the canvas's shape.
        let early = PictureLayout.videoRect(video: .zero, canvas: hd, in: CGSize(width: 1024, height: 768), fill: false)
        #expect(early == r)
    }

    /// With the controls up, the cues in the lower half move up together above the bottom block
    /// only when a control would cover them; top ones stay. Apple TV's transport bar: above it.
    @Test func liftClearOfControls() {
        let stage = CGRect(x: 0, y: 0, width: 1920, height: 1080)
        let bottom = [CGRect(x: 800, y: 900, width: 300, height: 40), CGRect(x: 800, y: 950, width: 300, height: 40)]
        let top = CGRect(x: 800, y: 50, width: 300, height: 40)
        let band = ControlsLayout.Part(kind: .bottom, frame: CGRect(x: 0, y: 920, width: 1920, height: 160))
        let centre = ControlsLayout.Part(kind: .centre, frame: CGRect(x: 900, y: 480, width: 120, height: 120), round: true)
        let controls = ControlsLayout(parts: [band, centre])
        #expect(abs(PictureLayout.lift(bottom + [top], stage: stage, controls: controls) - 78) < 0.001, "above the bottom block (920 - 8)")
        #expect(PictureLayout.lift([top], stage: stage, controls: controls) == 0, "a cue at the top stays")
        #expect(PictureLayout.lift(bottom, stage: stage) == 0, "controls hidden")
        let clear = [CGRect(x: 800, y: 700, width: 300, height: 40)]
        #expect(PictureLayout.lift(clear, stage: stage, controls: controls) == 0, "nothing covers it: it stays")
        #expect(abs(PictureLayout.lift(bottom, stage: stage, raisedBottom: 200) - 110) < 0.001, "above the transport bar")
        #expect(PictureLayout.lift(bottom, stage: stage, raisedBottom: 50) == 0)
    }
}
