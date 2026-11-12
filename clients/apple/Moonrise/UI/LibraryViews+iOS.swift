#if os(iOS)
import SwiftUI

/// A page whose navigation bar is hidden: UIKit's edge swipe back would be off (the navigation
/// controller refuses it without a bar). While the page shows, the swipe is allowed whenever
/// there's a page to go back to; the controller's own handling comes back after.
struct SwipeBackKeeper: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> Controller { Controller() }
    func updateUIViewController(_ vc: Controller, context: Context) {}

    final class Controller: UIViewController, UIGestureRecognizerDelegate {
        private var previous: [(UIGestureRecognizer, UIGestureRecognizerDelegate?)] = []
        private weak var nav: UINavigationController?

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            guard let nc = navigationController else { return }
            nav = nc
            #if DEBUG
            fputs("MoonriseSwipeBack: \(nc.viewControllers.count) pages\n", stderr)
            #endif
            // The edge swipe and iOS 26's swipe from anywhere in the content.
            for g in [nc.interactivePopGestureRecognizer, nc.interactiveContentPopGestureRecognizer].compactMap({ $0 }) where g.delegate !== self {
                previous.append((g, g.delegate))
                g.delegate = self
                g.isEnabled = true
            }
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            for (g, d) in previous where g.delegate === self { g.delegate = d }
            previous = []
        }

        func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
            (nav?.viewControllers.count ?? 0) > 1 && nav?.transitionCoordinator == nil
        }
    }
}
#endif
