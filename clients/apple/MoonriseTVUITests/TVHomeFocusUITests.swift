import XCTest

/// Apple TV Home focus, which only the focus engine decides. Runs the app's built-in demo on a
/// tvOS simulator: offline, no sign-in.
final class TVHomeFocusUITests: XCTestCase {
    private let app = XCUIApplication()
    private var focused: XCUIElement { app.buttons.element(matching: NSPredicate(format: "hasFocus == true")) }

    override func setUpWithError() throws { continueAfterFailure = false }

    /// The banner is full width, so Down went to the card nearest its centre.
    @MainActor func testDownFromTheBannerFocusesTheFirstCard() {
        app.launchEnvironment["MOONRISE_DEMO"] = "1"
        app.launch()
        // The demo's first film: exactly its name is its card (the banner's label is its whole summary).
        XCTAssertTrue(app.buttons["Sintel"].waitForExistence(timeout: 60), "Home's first row never appeared")

        // Focus starts on the tab bar; the banner is below it.
        XCUIRemote.shared.press(.down)
        waitForFocus("the banner") { $0.contains(",") }

        XCUIRemote.shared.press(.down)
        waitForFocus("a card of the first row") { !$0.contains(",") }
        XCTAssertEqual(focused.label, "Sintel")
    }

    private func waitForFocus(_ what: String, where matches: @escaping (String) -> Bool) {
        let landed = XCTNSPredicateExpectation(predicate: NSPredicate { [self] _, _ in
            focused.exists && matches(focused.label)
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [landed], timeout: 15), .completed, "Focus never reached \(what)")
    }
}
