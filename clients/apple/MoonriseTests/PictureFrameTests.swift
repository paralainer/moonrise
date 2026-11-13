// The Mac player's controls on the picture (PictureFrame in Moonrise/Player/PlayerView.swift), the
// subtitles above its bottom row (ControlsLayout.withBottomBand), and Download right after the
// title on the iPad's and Mac's title page (TitleAccessoryRow in Moonrise/UI/ItemDetailView.swift).
import CoreGraphics
import Testing
@testable import Moonrise

@Suite struct PictureFrameTests {
    /// A 4:3 episode in the Mac page's wide player: the picture is pillarboxed, and the controls
    /// take the picture, not the black bars beside it.
    @Test func pillarboxedControlsSitOnThePicture() {
        let stage = CGSize(width: 1000, height: 350)
        let video = CGSize(width: 960, height: 720)
        let controls = PictureFrame.controls(video: video, in: stage, fill: false)
        let width: CGFloat = 350 * 4 / 3
        #expect(abs(controls.width - width) < 0.001 && controls.height == 350)
        #expect(abs(controls.minX - (1000 - width) / 2) < 0.001 && controls.minY == 0)
    }

    /// A wide film in a taller stage: letterboxed, the controls on the picture too.
    @Test func letterboxedControlsSitOnThePicture() {
        let stage = CGSize(width: 1470, height: 956)
        let r = PictureFrame.controls(video: CGSize(width: 1920, height: 800), in: stage, fill: false)
        #expect(r.width == 1470)
        #expect(abs(r.height - 1470 * 800 / 1920) < 0.001)
        #expect(abs(r.midY - 478) < 0.001)
    }

    /// Fill, or a size not known yet (before the first frame): the whole stage, as before.
    @Test func fillOrUnknownSizeIsTheWholeStage() {
        let stage = CGSize(width: 1000, height: 350)
        let whole = CGRect(origin: .zero, size: stage)
        #expect(PictureFrame.controls(video: CGSize(width: 960, height: 720), in: stage, fill: true) == whole)
        #expect(PictureFrame.controls(video: .zero, in: stage, fill: false) == whole)
    }

    /// A narrow picture never squeezes the controls below their room; it widens about the
    /// picture's centre, and never past the stage.
    @Test func neverNarrowerThanTheControlsFit() {
        let stage = CGSize(width: 800, height: 300)
        let r = PictureFrame.controls(video: CGSize(width: 9, height: 16), in: stage, fill: false)
        #expect(r.width == PictureFrame.minControls.width)
        #expect(r.midX == 400)
        #expect(r.height == 300)
        let tiny = PictureFrame.controls(video: CGSize(width: 9, height: 16), in: CGSize(width: 300, height: 150), fill: false)
        #expect(tiny == CGRect(x: 0, y: 0, width: 300, height: 150))
    }

    /// The scrubber covered the second subtitle line on the Mac: with no bottom part reported,
    /// the band of the bottom row (where the controls are drawn) still lifts the text above it.
    @Test func subtitlesLiftAboveTheBottomRowWithoutItsReport() throws {
        let stage = CGRect(x: 0, y: 0, width: 1000, height: 350)
        let picture = PictureFrame.controls(video: CGSize(width: 960, height: 720), in: stage.size, fill: false)
        let insets = PlayerControls.rowInsets(compact: true)
        let layout = ControlsLayout().withBottomBand(of: picture, insets: insets)
        let band = try #require(layout.parts.first { $0.kind == .bottom })
        #expect(band.frame.maxY == picture.maxY - insets.height)
        #expect(band.frame.height == ControlsLayout.bottomRowHeight)

        // Two lines of the Mac's 28-point text.
        let box = TextBox(minX: 300, maxX: 700, height: 68)
        let p = SubtitleOverlay.bottomPadding(height: stage.height, baseSize: 28, raisedBottom: nil, controls: layout,
                                              liftedBottom: stage.height * 0.2, textBox: box, stage: stage,
                                              background: false, sizeScale: 1)
        #expect(stage.maxY - p <= band.frame.minY, "the text's bottom is under the scrubber row")
    }

    /// A reported bottom part is kept as it is: nothing is added.
    @Test func aReportedBottomRowIsKept() {
        let reported = ControlsLayout(parts: [.init(kind: .bottom, frame: CGRect(x: 0, y: 300, width: 1000, height: 40))])
        #expect(reported.withBottomBand(of: CGRect(x: 0, y: 0, width: 1000, height: 350), insets: CGSize(width: 10, height: 8)) == reported)
    }

    /// The title page's Download, playing: right after the title's text, centred on the block,
    /// not at the far end of the column (where it hung alone mid-window on the Mac).
    @Test func downloadFollowsTheTitle() {
        let f = TitleAccessoryRow.frames(title: CGSize(width: 260, height: 96), accessory: CGSize(width: 44, height: 44), spacing: 16)
        #expect(f.title == CGRect(x: 0, y: 0, width: 260, height: 96))
        #expect(f.accessory == CGRect(x: 276, y: 26, width: 44, height: 44))
    }
}
