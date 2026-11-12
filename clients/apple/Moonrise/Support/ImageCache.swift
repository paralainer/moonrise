import CryptoKit
import Foundation
import ImageIO
import SwiftUI

/// Two-level image cache (decoded images in memory, original bytes on disk) shared by every
/// RemoteImage. Keys ignore scheme/host/port and auth params, so artwork cached at home is still
/// a hit when the same server is reached through the Tailscale forwarder (127.0.0.1:port).
final class ImageCache: @unchecked Sendable {
    static let shared = ImageCache()

    private let memory = NSCache<NSString, PlatformImage>()
    private let directory: URL
    private let session: URLSession
    private let lock = NSLock()
    private var inflight: [String: Task<PlatformImage?, Never>] = [:]
    private static let diskLimit = 500 << 20

    private init() {
        memory.totalCostLimit = 150 << 20
        directory = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("images", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        session = HTTP.artworkSession()
        Task.detached(priority: .background) { [directory] in Self.trim(directory) }
    }

    static func key(_ url: URL) -> String {
        guard var c = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return url.absoluteString }
        c.queryItems = c.queryItems?.filter { !["apikey", "api_key"].contains($0.name.lowercased()) }
        return c.path + "?" + (c.percentEncodedQuery ?? "")
    }

    /// Instant lookup for first render, so cached artwork never flashes a placeholder.
    func cached(_ url: URL) -> PlatformImage? {
        memory.object(forKey: Self.key(url) as NSString)
    }

    /// Memory, else a synchronous disk read + decode. Only for the one hero image a page opens
    /// with, so a zoom transition lands on artwork instead of an empty frame.
    func cachedIncludingDisk(_ url: URL) -> PlatformImage? {
        if let hit = cached(url) { return hit }
        let key = Self.key(url)
        guard let data = try? Data(contentsOf: fileURL(key)), let img = Self.decode(data) else { return nil }
        memory.setObject(img, forKey: key as NSString, cost: Self.cost(img))
        return img
    }

    /// `headers`: for the few images that need the token (Fix Match's candidates).
    func image(for url: URL, headers: [String: String] = [:]) async -> PlatformImage? {
        let key = Self.key(url)
        if let hit = memory.object(forKey: key as NSString) { return hit }

        lock.lock()
        if let running = inflight[key] { lock.unlock(); return await running.value }
        let task = Task.detached(priority: .userInitiated) { [self] () -> PlatformImage? in
            // Artwork saved with a download: on the device already.
            if url.isFileURL { return (try? Data(contentsOf: url)).flatMap(Self.decode) }
            let file = fileURL(key)
            if let data = try? Data(contentsOf: file), let img = Self.decode(data) {
                try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: file.path) // LRU
                return img
            }
            let answer = headers.isEmpty
                ? try? await HTTP.data(from: url, on: session)
                : try? await HTTP.data(for: HTTP.request(url, headers: headers), on: session)
            guard let (data, status) = answer, status == 200, let img = Self.decode(data) else { return nil }
            try? data.write(to: file, options: .atomic)
            return img
        }
        inflight[key] = task
        lock.unlock()

        let img = await task.value
        lock.lock(); inflight[key] = nil; lock.unlock()
        if let img { memory.setObject(img, forKey: key as NSString, cost: Self.cost(img)) }
        return img
    }

    /// Download to disk only (no decode, no memory), for images likely to be needed soon.
    func prefetch(_ url: URL) async {
        let key = Self.key(url)
        let file = fileURL(key)
        guard memory.object(forKey: key as NSString) == nil,
              !FileManager.default.fileExists(atPath: file.path),
              let (data, status) = try? await HTTP.data(from: url, on: session), status == 200 else { return }
        try? data.write(to: file, options: .atomic)
    }

    private func fileURL(_ key: String) -> URL {
        let hash = SHA256.hash(data: Data(key.utf8)).map { String(format: "%02x", $0) }.joined()
        return directory.appendingPathComponent(hash)
    }

    /// Decode off the main thread so scrolling never stalls on JPEG decompression.
    private static func decode(_ data: Data) -> PlatformImage? {
        #if os(macOS)
        return NSImage(data: data)
        #else
        guard let img = UIImage(data: data) else { return nil }
        return img.preparingForDisplay() ?? img
        #endif
    }

    private static func cost(_ img: PlatformImage) -> Int {
        #if os(macOS)
        return Int(img.size.width * img.size.height * 4)
        #else
        return Int(img.size.width * img.scale * img.size.height * img.scale * 4)
        #endif
    }

    /// Drop least-recently-used files once the disk cache passes its limit.
    private static func trim(_ dir: URL) {
        let keys: [URLResourceKey] = [.contentModificationDateKey, .totalFileAllocatedSizeKey]
        guard let files = try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: keys) else { return }
        var entries = files.compactMap { f -> (URL, Date, Int)? in
            guard let v = try? f.resourceValues(forKeys: Set(keys)) else { return nil }
            return (f, v.contentModificationDate ?? .distantPast, v.totalFileAllocatedSize ?? 0)
        }
        var total = entries.reduce(0) { $0 + $1.2 }
        guard total > diskLimit else { return }
        entries.sort { $0.1 < $1.1 }
        for (f, _, size) in entries where total > diskLimit * 8 / 10 {
            try? FileManager.default.removeItem(at: f)
            total -= size
        }
    }
}
