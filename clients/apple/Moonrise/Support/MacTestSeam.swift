#if DEBUG && os(macOS)
import AppKit
import SwiftUI

/// Test seam for scripted UI checks over SSH (no Screen Recording or Accessibility permission
/// there): launch with MOONRISE_MAC_SEAM=1 and write commands, one per line, to
/// `~/Library/Containers/<bundle id>/Data/seam/cmd`. Results go to `seam/log`,
/// screenshots to `seam/<name>.png`. Debug builds only.
///
///   shot NAME            PNG of the app's on-screen windows (menus included)
///   size W H             the window's size (top-left stays)   | frame X Y W H (screen, top-left origin)
///   fs                   toggle the window's full screen      | info: window frame, full screen, responder
///   key SPEC             e.g. space, left, esc, f, cmd+f, ctrl+cmd+f, cmd+1, shift+cmd+[
///   move X Y | click X Y | dclick X Y | rclick X Y   (window points, top-left origin)
///   scroll DX DY X Y     scroll wheel (points) at X Y
///   menu A>B             performs the menu bar item A > B
///   wait SECONDS
@MainActor
enum MacTestSeam {
    static let dir = URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("seam")
    private static var timer: Timer?
    private static var busy = false

    static func start() {
        guard DebugSeams.launch.macSeam, timer == nil else { return }
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        log("seam ready")
        let t = Timer(timeInterval: 0.25, repeats: true) { _ in
            MainActor.assumeIsolated { poll() }
        }
        RunLoop.main.add(t, forMode: .common) // also while a menu tracks
        timer = t
    }

    private static func poll() {
        guard !busy else { return }
        let file = dir.appendingPathComponent("cmd")
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return }
        try? FileManager.default.removeItem(at: file)
        busy = true
        Task { @MainActor in
            for line in text.split(separator: "\n").map({ $0.trimmingCharacters(in: .whitespaces) }) where !line.isEmpty {
                await run(line)
            }
            log("done")
            busy = false
        }
    }

    private static func log(_ s: String) {
        let url = dir.appendingPathComponent("log")
        let line = "\(Date().timeIntervalSince1970) \(s)\n"
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile(); h.write(line.data(using: .utf8)!); try? h.close()
        } else {
            try? line.write(to: url, atomically: true, encoding: .utf8)
        }
    }

    private static var window: NSWindow? {
        NSApp.keyWindow ?? NSApp.mainWindow ?? NSApp.windows.first { $0.isVisible && $0.canBecomeMain }
    }

    private static func run(_ line: String) async {
        let parts = line.split(separator: " ").map(String.init)
        let args = Array(parts.dropFirst())
        let n = args.compactMap { Double($0) }
        switch parts[0] {
        case "shot":
            shot(args.first ?? "shot", title: args.count > 1 ? args.dropFirst().joined(separator: " ") : nil)
        case "size":
            guard n.count == 2, let w = window else { break }
            var f = w.frame
            let top = f.maxY
            f.size = NSSize(width: n[0], height: n[1])
            f.origin.y = top - n[1]
            w.setFrame(f, display: true, animate: false)
        case "frame":
            guard n.count == 4, let w = window, let screen = w.screen ?? NSScreen.main else { break }
            let y = screen.frame.maxY - n[1] - n[3]
            w.setFrame(NSRect(x: n[0], y: y, width: n[2], height: n[3]), display: true, animate: false)
        case "fs":
            window?.toggleFullScreen(nil)
        case "windows":
            for w in NSApp.windows where w.isVisible { log("window '\(w.title)' \(w.frame) key=\(w.isKeyWindow)") }
        case "info":
            if let w = window {
                log("info key=\(w.isKeyWindow) active=\(NSApp.isActive) frame=\(w.frame) content=\(w.contentLayoutRect.size) fullScreen=\(w.styleMask.contains(.fullScreen)) screen=\(w.screen?.frame ?? .zero) responder=\(String(describing: w.firstResponder.map { type(of: $0) })) windows=\(NSApp.windows.filter(\.isVisible).count)")
            }
        case "key":
            if let spec = args.first { key(spec) }
        case "move", "click", "dclick", "rclick":
            guard n.count == 2 else { break }
            mouse(parts[0], NSPoint(x: n[0], y: n[1]))
        case "scroll":
            guard n.count == 4 else { break }
            scroll(dx: n[0], dy: n[1], at: NSPoint(x: n[2], y: n[3]))
        case "menu":
            menu(args.joined(separator: " "))
        case "wids":
            // Moonrise's on-screen windows, front to back: id and bounds (screen points, top-left origin).
            let pid = ProcessInfo.processInfo.processIdentifier
            let info = (CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]]) ?? []
            for w in info where (w[kCGWindowOwnerPID as String] as? Int32) == pid {
                let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
                log("wid \(w[kCGWindowNumber as String] ?? 0) \(Int(b["X"] ?? 0)),\(Int(b["Y"] ?? 0)),\(Int(b["Width"] ?? 0)),\(Int(b["Height"] ?? 0)) layer=\(w[kCGWindowLayer as String] ?? 0)")
            }
        case "hub":
            // The Playback menu's actions, without the menu (it can't be opened over SSH).
            let hub = MacPlayerHub.shared
            switch args.first {
            case "play": hub.togglePlay()
            case "fwd": hub.skip(true)
            case "back": hub.skip(false)
            case "up": hub.changeVolume(0.1)
            case "down": hub.changeVolume(-0.1)
            case "mute": hub.toggleMute()
            case "fs": hub.toggleFullScreen?()
            default: log("hub ?")
            }
        case "winfs":
            MacWindow.shared.setFullScreen(!MacWindow.shared.isFullScreen)
        case "state":
            log("state \(MacPlayerHub.shared.debugDescription)")
        case "menus":
            dumpMenus()
        case "wait":
            try? await Task.sleep(for: .seconds(n.first ?? 1))
        default:
            log("unknown \(line)")
        }
        // Let the UI settle between commands.
        try? await Task.sleep(for: .milliseconds(120))
    }

    // MARK: Capture

    private typealias CreateImageFromArray = @convention(c) (CGRect, CFArray, UInt32) -> Unmanaged<CGImage>?

    private static func shot(_ name: String, title: String? = nil) {
        let pid = ProcessInfo.processInfo.processIdentifier
        let info = (CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]]) ?? []
        // Front to back: the array wants them in that order too.
        let mine = info.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowAlpha as String] as? Double ?? 1) > 0 }
        let ids = mine.compactMap { $0[kCGWindowNumber as String] as? UInt32 }
        let url = dir.appendingPathComponent("\(name).png")
        if title == nil, !ids.isEmpty, let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImageFromArray") {
            let f = unsafeBitCast(sym, to: CreateImageFromArray.self)
            let arr = ids.map { NSNumber(value: $0) } as CFArray
            // .boundsIgnoreFraming | .bestResolution
            if let img = f(.null, arr, (1 << 0) | (1 << 3))?.takeRetainedValue(), img.width > 4 {
                write(NSBitmapImageRep(cgImage: img), to: url)
                log("shot \(name) \(img.width)x\(img.height) windows=\(ids.count)")
                return
            }
        }
        // Fallback: draw the window's views (no video layer, no menus).
        let target = title.flatMap { t in NSApp.windows.first { $0.isVisible && $0.title.localizedCaseInsensitiveContains(t) } } ?? window
        guard let w = target, let view = w.contentView?.superview ?? w.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { log("shot failed"); return }
        view.cacheDisplay(in: view.bounds, to: rep)
        write(rep, to: url)
        log("shot \(name) (views) \(rep.pixelsWide)x\(rep.pixelsHigh)")
    }

    private static func write(_ rep: NSBitmapImageRep, to url: URL) {
        try? rep.representation(using: .png, properties: [:])?.write(to: url)
    }

    // MARK: Events

    private static let codes: [String: UInt16] = [
        "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12,
        "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23,
        "=": 24, "9": 25, "7": 26, "-": 27, "8": 28, "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34,
        "p": 35, "l": 37, "j": 38, "k": 40, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47,
        "return": 36, "tab": 48, "space": 49, "delete": 51, "esc": 53,
        "left": 123, "right": 124, "down": 125, "up": 126,
    ]

    private static func key(_ spec: String) {
        var flags: NSEvent.ModifierFlags = []
        var name = spec
        let pieces = spec.split(separator: "+").map(String.init)
        if pieces.count > 1 {
            name = pieces.last!
            for m in pieces.dropLast() {
                switch m {
                case "cmd": flags.insert(.command)
                case "shift": flags.insert(.shift)
                case "ctrl": flags.insert(.control)
                case "opt", "alt": flags.insert(.option)
                default: break
                }
            }
        }
        guard let code = codes[name], let w = window else { log("bad key \(spec)"); return }
        var chars: String
        switch name {
        case "space": chars = " "
        case "return": chars = "\r"
        case "tab": chars = "\t"
        case "delete": chars = "\u{7f}"
        case "esc": chars = "\u{1b}"
        case "left": chars = String(Character(UnicodeScalar(NSLeftArrowFunctionKey)!)); flags.formUnion([.function, .numericPad])
        case "right": chars = String(Character(UnicodeScalar(NSRightArrowFunctionKey)!)); flags.formUnion([.function, .numericPad])
        case "up": chars = String(Character(UnicodeScalar(NSUpArrowFunctionKey)!)); flags.formUnion([.function, .numericPad])
        case "down": chars = String(Character(UnicodeScalar(NSDownArrowFunctionKey)!)); flags.formUnion([.function, .numericPad])
        default: chars = name
        }
        let unmod = chars
        if flags.contains(.shift), chars.count == 1, chars.first!.isLetter { chars = chars.uppercased() }
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            if let e = NSEvent.keyEvent(with: type, location: .zero, modifierFlags: flags, timestamp: ProcessInfo.processInfo.systemUptime,
                                        windowNumber: w.windowNumber, context: nil, characters: chars,
                                        charactersIgnoringModifiers: unmod, isARepeat: false, keyCode: code) {
                NSApp.postEvent(e, atStart: false)
            }
        }
        log("key \(spec)")
    }

    private static func mouse(_ kind: String, _ p: NSPoint) {
        guard let w = window else { return }
        let loc = NSPoint(x: p.x, y: w.frame.height - p.y)
        func post(_ type: NSEvent.EventType, clicks: Int = 1) {
            if let e = NSEvent.mouseEvent(with: type, location: loc, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                          windowNumber: w.windowNumber, context: nil, eventNumber: 0, clickCount: clicks, pressure: 1) {
                NSApp.postEvent(e, atStart: false)
            }
        }
        // Put the real cursor there too, so hover tracking agrees.
        let screenPoint = w.convertPoint(toScreen: loc)
        if let main = NSScreen.screens.first {
            CGWarpMouseCursorPosition(CGPoint(x: screenPoint.x, y: main.frame.maxY - screenPoint.y))
        }
        switch kind {
        case "move": post(.mouseMoved, clicks: 0)
        case "click": post(.leftMouseDown); post(.leftMouseUp)
        case "dclick": post(.leftMouseDown); post(.leftMouseUp); post(.leftMouseDown, clicks: 2); post(.leftMouseUp, clicks: 2)
        case "rclick": post(.rightMouseDown); post(.rightMouseUp)
        default: break
        }
        log("\(kind) \(p)")
    }

    private static func scroll(dx: Double, dy: Double, at p: NSPoint) {
        guard let w = window, let main = NSScreen.screens.first else { return }
        let loc = w.convertPoint(toScreen: NSPoint(x: p.x, y: w.frame.height - p.y))
        CGWarpMouseCursorPosition(CGPoint(x: loc.x, y: main.frame.maxY - loc.y))
        // A short trackpad-like gesture: several small precise deltas.
        let steps = 8
        for i in 0..<steps {
            guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2,
                                   wheel1: Int32(dy / Double(steps)), wheel2: Int32(dx / Double(steps)), wheel3: 0) else { continue }
            cg.location = CGPoint(x: loc.x, y: main.frame.maxY - loc.y)
            cg.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            cg.setIntegerValueField(.scrollWheelEventScrollPhase, value: Int64(i == 0 ? 1 : i == steps - 1 ? 4 : 2))
            cg.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(w.windowNumber))
            cg.setIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent, value: Int64(w.windowNumber))
            if let e = NSEvent(cgEvent: cg) { w.sendEvent(e) }
        }
        log("scroll \(dx),\(dy) at \(p)")
    }

    private static func menu(_ path: String) {
        let names = path.split(separator: ">").map { $0.trimmingCharacters(in: .whitespaces) }
        var menu = NSApp.mainMenu
        for (i, name) in names.enumerated() {
            guard let item = menu?.items.first(where: { $0.title == name }) else {
                log("menu: no \(name) in \(menu?.items.map(\.title) ?? [])"); return
            }
            if i == names.count - 1, let m = menu {
                m.performActionForItem(at: m.index(of: item))
                log("menu \(path)")
            } else {
                menu = item.submenu
                menu?.update()
            }
        }
    }

    /// The menu bar, for checking what's there.
    static func dumpMenus() {
        for top in NSApp.mainMenu?.items ?? [] {
            let items = top.submenu?.items.map { $0.isSeparatorItem ? "—" : "\($0.title)[\($0.keyEquivalentModifierMask.rawValue >> 17):\($0.keyEquivalent)]" } ?? []
            log("menu \(top.title): \(items.joined(separator: ", "))")
        }
    }
}
#endif
