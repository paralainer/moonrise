// Where subtitles sit while the player's controls show (SubtitleOverlay.bottomPadding in
// Moonrise/Player/PlayerView.swift), and the minimum lift while they're hidden.
import CoreGraphics
import Testing
@testable import Moonrise

private typealias Part = ControlsLayout.Part

/// The padding for a text box `textBox` on `stage` with these controls showing.
private func padding(stage: CGRect, textBox: TextBox, parts: [Part]?, height: CGFloat? = nil, baseSize: CGFloat = 22,
                     liftedBottom: CGFloat = 0, raised: CGFloat? = nil) -> CGFloat {
    SubtitleOverlay.bottomPadding(height: height ?? stage.height, baseSize: baseSize, raisedBottom: raised,
                                  controls: parts.map { ControlsLayout(parts: $0) }, liftedBottom: liftedBottom,
                                  textBox: textBox, stage: stage, background: false, sizeScale: 1)
}

/// The text's frame on the stage for that padding.
private func frame(stage: CGRect, textBox: TextBox, padding: CGFloat) -> CGRect {
    let bottom = stage.maxY - padding
    return CGRect(x: textBox.minX, y: bottom - textBox.height, width: textBox.maxX - textBox.minX, height: textBox.height)
}

@Suite struct SubtitlePlacementTests {
    /// Controls hidden: 6 % of the picture, at least the text's base size (a small player
    /// on the phone's title page kept the text on its bottom edge).
    @Test func controlsHidden() {
        let box = TextBox(minX: 100, maxX: 300, height: 30)
        #expect(padding(stage: CGRect(x: 0, y: 0, width: 1920, height: 1080), textBox: box, parts: nil) == 1080 * 0.06)
        #expect(padding(stage: CGRect(x: 0, y: 0, width: 390, height: 220), textBox: box, parts: nil, baseSize: 18) == 18)
        // The Apple TV's transport bar sets it outright.
        #expect(padding(stage: CGRect(x: 0, y: 0, width: 1920, height: 1080), textBox: box, parts: [], raised: 200) == 200)
    }

    /// iPhone landscape, two lines, controls up. The text mustn't sit under the round
    /// centre buttons or inside the bottom block: it goes to a clear spot above the centre buttons.
    @Test func iPhoneLandscapeClearOfTheCentreButtons() {
        let stage = CGRect(x: 0, y: 0, width: 844, height: 390)
        let centre = Part(kind: .centre, frame: CGRect(x: 362, y: 135, width: 120, height: 120), round: true)
        let bottom = Part(kind: .bottom, frame: CGRect(x: 0, y: 300, width: 844, height: 90))
        let top = Part(kind: .top, frame: CGRect(x: 0, y: 0, width: 844, height: 44))
        for lines in 1...3 {
            let box = TextBox(minX: 222, maxX: 622, height: CGFloat(lines) * 28)
            let p = padding(stage: stage, textBox: box, parts: [top, centre, bottom])
            let r = frame(stage: stage, textBox: box, padding: p)
            #expect(!centre.hides(r), "\(lines) line(s) under the centre buttons")
            #expect(!top.hides(r), "\(lines) line(s) under the top bar")
            #expect(r.maxY <= bottom.frame.minY, "\(lines) line(s) inside the bottom block")
            #expect(r.minY >= stage.minY)
        }
    }

    /// iPad landscape. The bottom block (title, track pills, scrubber) counts as one
    /// band: the text never slips in between its rows.
    @Test func iPadBottomBlockIsOneBand() {
        let stage = CGRect(x: 0, y: 0, width: 1194, height: 834)
        let title = Part(kind: .bottom, frame: CGRect(x: 40, y: 640, width: 500, height: 44))
        let pills = Part(kind: .bottom, frame: CGRect(x: 40, y: 740, width: 700, height: 40))
        let box = TextBox(minX: 397, maxX: 797, height: 40)
        let p = padding(stage: stage, textBox: box, parts: [title, pills])
        let r = frame(stage: stage, textBox: box, padding: p)
        #expect(r.maxY <= title.frame.minY)
        #expect(!title.hides(r) && !pills.hides(r))
    }

    /// The page's small portrait player with big text: nothing is clear, and the old
    /// fixed lift stays unless a spot hides less of the letters; it never leaves the stage.
    @Test func nothingClearNeverWorseThanTheOldLift() {
        let stage = CGRect(x: 0, y: 0, width: 390, height: 220)
        let centre = Part(kind: .centre, frame: CGRect(x: 155, y: 70, width: 80, height: 80), round: true)
        let bottom = Part(kind: .bottom, frame: CGRect(x: 0, y: 170, width: 390, height: 50))
        let box = TextBox(minX: 40, maxX: 350, height: 120)
        let lifted: CGFloat = 220 * 0.2
        let p = padding(stage: stage, textBox: box, parts: [centre, bottom], baseSize: 18, liftedBottom: lifted)
        let r = frame(stage: stage, textBox: box, padding: p)
        #expect(r.minY >= stage.minY - 0.001)
        let candidates: [CGFloat] = [stage.maxY - lifted, bottom.frame.minY - 8, centre.frame.minY - 4]
            .map { max($0, stage.minY + box.height) }
        #expect(candidates.contains(r.maxY))
    }

    /// No measured text yet: where it is with the controls hidden.
    @Test func unmeasuredText() {
        let stage = CGRect(x: 0, y: 0, width: 844, height: 390)
        let parts = [Part(kind: .bottom, frame: CGRect(x: 0, y: 300, width: 844, height: 90))]
        #expect(padding(stage: stage, textBox: TextBox(), parts: parts) == max(390 * 0.06, 22))
    }
}
