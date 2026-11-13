import SwiftUI

/// Picture subtitles (PGS, DVD, DVB) over the video: each cue's image where the track puts it,
/// its canvas stretched over the rectangle the video is drawn in (`PictureLayout`). Drawn on
/// the same stage as the text overlay; while the controls (or Apple TV's transport bar) would
/// cover the cues in the lower half, those move up above them as one block, as text does.
struct PictureSubtitleOverlay: View {
    let cues: [PictureCue]
    let canvas: CGSize
    /// The video's picture size as it shows (zero until known: the canvas's shape then).
    let videoSize: CGSize
    /// The video fills the stage (cropped) rather than fitting in it.
    var fill = false
    /// Our controls are showing, here (`ControlsLayout.space`).
    var controls: ControlsLayout? = nil
    /// Apple TV's system transport bar is showing: the cues end this far above the bottom.
    var raisedBottom: CGFloat? = nil
    /// This overlay's frame on the stage.
    @State private var stage: CGRect = .zero

    var body: some View {
        GeometryReader { geo in
            let rect = PictureLayout.videoRect(video: videoSize, canvas: canvas, in: geo.size, fill: fill)
            let frames = cues.map { PictureLayout.frame($0.rect, canvas: canvas, videoRect: rect) }
            // The controls' frames are on the stage: the cues' too, for the comparison.
            let origin = controls != nil ? stage.origin : .zero
            let lift = PictureLayout.lift(frames.map { $0.offsetBy(dx: origin.x, dy: origin.y) },
                                          stage: CGRect(origin: origin, size: geo.size),
                                          controls: controls, raisedBottom: raisedBottom)
            ZStack(alignment: .topLeading) {
                ForEach(Array(zip(cues, frames).enumerated()), id: \.offset) { _, pair in
                    if let image = pair.0.image {
                        let f = pair.1
                        Image(decorative: image, scale: 1)
                            .resizable()
                            .interpolation(.high)
                            .frame(width: f.width, height: f.height)
                            .position(x: f.midX, y: f.midY - (f.midY > geo.size.height / 2 ? lift : 0))
                    }
                }
            }
            .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
            .animation(.easeInOut(duration: 0.2), value: lift)
        }
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .named(ControlsLayout.space)) } action: { stage = $0 }
        .allowsHitTesting(false)
    }
}
