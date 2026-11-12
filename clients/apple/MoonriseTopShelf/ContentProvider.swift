import TVServices

/// The Top Shelf above Moonrise when it's in the Apple TV's top row: Continue Watching, as the
/// app last saved it (`TopShelfSnapshot`). No snapshot (signed out, the demo, never
/// opened): nil, and the system shows the static Top Shelf image.
final class ContentProvider: TVTopShelfContentProvider {
    override func loadTopShelfContent(completionHandler: @escaping (TVTopShelfContent?) -> Void) {
        completionHandler(Self.content())
    }

    static func content() -> TVTopShelfContent? {
        guard let snapshot = TopShelfShared.load() else { return nil }
        let sections: [TVTopShelfItemCollection<TVTopShelfSectionedItem>] = snapshot.sections.compactMap { section in
            let items = section.items.map(item(for:))
            guard !items.isEmpty else { return nil }
            let collection = TVTopShelfItemCollection(items: items)
            collection.title = section.title
            return collection
        }
        return sections.isEmpty ? nil : TVTopShelfSectionedContent(sections: sections)
    }

    private static func item(for entry: TopShelfSnapshot.Item) -> TVTopShelfSectionedItem {
        let item = TVTopShelfSectionedItem(identifier: entry.id)
        item.title = entry.title
        item.imageShape = .hdtv
        if let url = entry.image {
            // One picture, wide enough for a 4K screen's cards.
            item.setImageURL(url, for: .screenScale1x)
            item.setImageURL(url, for: .screenScale2x)
        }
        if let p = entry.progress { item.playbackProgress = min(max(p, 0), 1) }
        item.displayAction = TVTopShelfAction(url: TopShelfShared.Link.open(entry.id).url)
        item.playAction = TVTopShelfAction(url: TopShelfShared.Link.play(entry.id).url)
        return item
    }
}
