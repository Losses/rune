import Foundation
import ImageIO
import MusicKit
import UniformTypeIdentifiers

/// The caller's completion: its context pointer and a NUL-terminated UTF-8 JSON reply that is
/// only valid during the call.
public typealias AppleMusicReply = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>) -> Void

// The context pointer belongs to the Rust caller, which hands it back to its own completion;
// Swift never reads through it.
private struct ReplyTarget: @unchecked Sendable {
    let context: UnsafeMutableRawPointer?
    let reply: AppleMusicReply

    func send(_ value: [String: Any]) {
        let data = (try? JSONSerialization.data(withJSONObject: value)) ?? Data("{\"error\":{\"kind\":\"internal\",\"detail\":\"unencodable reply\"}}".utf8)
        String(decoding: data, as: UTF8.self).withCString { reply(context, $0) }
    }

    func fail(_ kind: String, _ detail: String) {
        send(["error": ["kind": kind, "detail": detail]])
    }
}

/// Answers one Apple Music request from the hub. `request` is a JSON object with an `op` and its
/// arguments; the reply is `{"ok": ...}` or `{"error": {"kind", "detail"}}`, delivered once on an
/// arbitrary thread.
///
/// Library ops: `songs`, `albums`, `artists`, `playlists` (each with `offset`, `limit`), `song`,
/// `album`, `artist`, `playlist` (each with `id`) and `search` (`term`, `limit`). Player ops
/// (`playerSet`, `playerAdd`, `playerClear`, `playerRemove`, `playerSkip`, `playerStart`,
/// `playerStop`, `playerGet`) all reply with the player's state: `playing`, `position` in
/// seconds, `currentIndex` when there is a current entry, and the queued song `ids`.
@c(rune_apple_music_request)
public func runeAppleMusicRequest(
    _ request: UnsafePointer<CChar>,
    _ context: UnsafeMutableRawPointer?,
    _ reply: AppleMusicReply
) {
    let target = ReplyTarget(context: context, reply: reply)
    let text = String(cString: request)
    guard #available(macOS 14.0, iOS 16.0, *) else {
        target.fail("unsupported", "Apple Music needs macOS 14 or later")
        return
    }
    guard let data = text.data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data),
          let arguments = object as? [String: Any],
          let operation = arguments["op"] as? String
    else {
        target.fail("invalid", "the request is not a JSON object with an op")
        return
    }
    let call = AppleMusicCall(operation: operation, arguments: AppleMusicArguments(arguments))
    Task {
        do {
            target.send(["ok": try await call.run()])
        } catch let error as AppleMusicFailure {
            target.fail(error.kind, error.detail)
        } catch {
            target.fail("service", String(describing: error))
        }
    }
}

private struct AppleMusicFailure: Error {
    let kind: String
    let detail: String
}

// The arguments are plain JSON values, copied out before the task starts.
private struct AppleMusicArguments: @unchecked Sendable {
    private let values: [String: Any]

    init(_ values: [String: Any]) {
        self.values = values
    }

    func int(_ key: String, default fallback: Int) -> Int {
        (values[key] as? NSNumber)?.intValue ?? fallback
    }

    func double(_ key: String, default fallback: Double) -> Double {
        (values[key] as? NSNumber)?.doubleValue ?? fallback
    }

    func int(_ key: String) throws -> Int {
        guard let value = values[key] as? NSNumber else {
            throw AppleMusicFailure(kind: "invalid", detail: "missing \(key)")
        }
        return value.intValue
    }

    func string(_ key: String) throws -> String {
        guard let value = values[key] as? String else {
            throw AppleMusicFailure(kind: "invalid", detail: "missing \(key)")
        }
        return value
    }

    func strings(_ key: String) throws -> [String] {
        guard let value = values[key] as? [String] else {
            throw AppleMusicFailure(kind: "invalid", detail: "missing \(key)")
        }
        return value
    }
}

@available(macOS 14.0, iOS 16.0, *)
private struct AppleMusicCall: Sendable {
    let operation: String
    let arguments: AppleMusicArguments

    func run() async throws -> Any {
        switch operation {
        case "status":
            return ["authorization": name(MusicAuthorization.currentStatus)]
        case "authorize":
            return ["authorization": name(await MusicAuthorization.request())]
        default:
            break
        }
        guard MusicAuthorization.currentStatus == .authorized else {
            throw AppleMusicFailure(kind: "notAuthorized", detail: "Apple Music access is not allowed")
        }
        let offset = arguments.int("offset", default: 0)
        let limit = arguments.int("limit", default: 100)
        let jukebox = AppleMusicJukebox.shared
        switch operation {
        case "songCount":
            // MusicKit has no count request, so the pages are walked without encoding them.
            var total = 0
            while true {
                var request = MusicLibraryRequest<Song>()
                request.offset = total
                request.limit = 1000
                let count = try await request.response().items.count
                total += count
                if count < 1000 { return ["count": total] }
            }
        case "songs":
            var request = MusicLibraryRequest<Song>()
            request.sort(by: \.title, ascending: true)
            request.offset = offset
            request.limit = limit
            return try await request.response().items.map(song)
        case "albums":
            var request = MusicLibraryRequest<Album>()
            request.sort(by: \.title, ascending: true)
            request.offset = offset
            request.limit = limit
            return try await request.response().items.map(album)
        case "artists":
            var request = MusicLibraryRequest<Artist>()
            request.sort(by: \.name, ascending: true)
            request.offset = offset
            request.limit = limit
            return try await request.response().items.map(artist)
        case "playlists":
            var request = MusicLibraryRequest<Playlist>()
            request.sort(by: \.name, ascending: true)
            request.offset = offset
            request.limit = limit
            return try await request.response().items.map(playlist)
        case "artwork":
            let id = MusicItemID(try arguments.string("id"))
            let size = max(64, min(arguments.int("size", default: 300), 1200))
            var artworks: [Artwork] = []
            if try arguments.string("kind") == "album" {
                var request = MusicLibraryRequest<Album>()
                request.filter(matching: \.id, equalTo: id)
                if let album = try await request.response().items.first {
                    if let artwork = album.artwork { artworks.append(artwork) }
                    // An album's own artwork can come back empty; its songs carry the same picture.
                    if let first = try await album.with([.tracks]).tracks?.first, let artwork = first.artwork {
                        artworks.append(artwork)
                    }
                }
            } else {
                var request = MusicLibraryRequest<Song>()
                request.filter(matching: \.id, equalTo: id)
                if let artwork = try await request.response().items.first?.artwork { artworks.append(artwork) }
            }
            for artwork in artworks {
                // MusicKit serves these addresses itself, as full-size PNGs.
                guard let url = artwork.url(width: size, height: size),
                      let (data, _) = try? await URLSession.shared.data(from: url),
                      let jpeg = scaledJPEG(data, size: size)
                else { continue }
                return ["jpeg": jpeg.base64EncodedString()]
            }
            throw AppleMusicFailure(kind: "notFound", detail: "no artwork")
        case "songsById":
            return try await librarySongs(try arguments.strings("ids")).map(song)
        case "song":
            var request = MusicLibraryRequest<Song>()
            request.filter(matching: \.id, equalTo: MusicItemID(try arguments.string("id")))
            guard let found = try await request.response().items.first else {
                throw AppleMusicFailure(kind: "notFound", detail: "no such song")
            }
            return song(found)
        case "album":
            var request = MusicLibraryRequest<Album>()
            request.filter(matching: \.id, equalTo: MusicItemID(try arguments.string("id")))
            guard let found = try await request.response().items.first else {
                throw AppleMusicFailure(kind: "notFound", detail: "no such album")
            }
            var value = album(found)
            let tracks = try await found.with([.tracks]).tracks ?? []
            // A song read through its album belongs to it even when MusicKit has not loaded the
            // song's own album relationship.
            value["songs"] = tracks.compactMap(track).map { song in
                song.merging(["albumId": found.id.rawValue]) { own, _ in own }
            }
            return value
        case "artist":
            var request = MusicLibraryRequest<Artist>()
            request.filter(matching: \.id, equalTo: MusicItemID(try arguments.string("id")))
            guard let found = try await request.response().items.first else {
                throw AppleMusicFailure(kind: "notFound", detail: "no such artist")
            }
            var value = artist(found)
            value["albums"] = try await found.with([.albums]).albums?.map(album) ?? []
            return value
        case "playlist":
            var request = MusicLibraryRequest<Playlist>()
            request.filter(matching: \.id, equalTo: MusicItemID(try arguments.string("id")))
            guard let found = try await request.response().items.first else {
                throw AppleMusicFailure(kind: "notFound", detail: "no such playlist")
            }
            var value = playlist(found)
            value["songs"] = try await found.with([.tracks]).tracks?.compactMap(track) ?? []
            return value
        case "search":
            var request = MusicLibrarySearchRequest(
                term: try arguments.string("term"),
                types: [Song.self, Album.self, Artist.self]
            )
            request.limit = limit
            let response = try await request.response()
            return [
                "songs": response.songs.map(song),
                "albums": response.albums.map(album),
                "artists": response.artists.map(artist),
            ]
        case "playerSet":
            let songs = try await librarySongs(try arguments.strings("ids"))
            return await jukebox.set(songs).json
        case "playerAdd":
            let songs = try await librarySongs(try arguments.strings("ids"))
            return try await jukebox.add(songs).json
        case "playerMode":
            return try await jukebox.setMode(try arguments.string("mode")).json
        case "playerClear":
            return await jukebox.clear().json
        case "playerRemove":
            return try await jukebox.remove(at: try arguments.int("index")).json
        case "playerSkip":
            return try await jukebox.skip(
                to: try arguments.int("index"),
                offset: arguments.double("offset", default: 0)
            ).json
        case "playerStart":
            return try await jukebox.start().json
        case "playerStop":
            return await jukebox.stop().json
        case "playerGet":
            return await jukebox.snapshot().json
        default:
            throw AppleMusicFailure(kind: "invalid", detail: "unknown op \(operation)")
        }
    }

    /// The library songs with these ids, in the same order and with repeats kept. Ids that are no
    /// longer in the library are left out.
    private func librarySongs(_ ids: [String]) async throws -> [Song] {
        let unique = Array(Set(ids))
        guard !unique.isEmpty else { return [] }
        // A whole library can be queued at once; MusicKit is asked for it in batches.
        var found: [Song] = []
        var start = 0
        while start < unique.count {
            let batch = unique[start..<min(start + 200, unique.count)]
            var request = MusicLibraryRequest<Song>()
            request.filter(matching: \.id, memberOf: batch.map { MusicItemID($0) })
            request.limit = batch.count
            found.append(contentsOf: try await request.response().items)
            start += batch.count
        }
        let byId = Dictionary(found.map { ($0.id.rawValue, $0) }) { first, _ in first }
        return ids.compactMap { byId[$0] }
    }

    // Covers are shown small and cached by the hub, so they are scaled down and compressed here.
    private func scaledJPEG(_ data: Data, size: Int) -> Data? {
        guard !data.isEmpty, let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: size,
            kCGImageSourceCreateThumbnailWithTransform: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(output, UTType.jpeg.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        return CGImageDestinationFinalize(destination) ? output as Data : nil
    }

    private func name(_ status: MusicAuthorization.Status) -> String {
        switch status {
        case .authorized: return "authorized"
        case .denied: return "denied"
        case .restricted: return "restricted"
        case .notDetermined: return "notDetermined"
        @unknown default: return "unknown"
        }
    }

    private func song(_ song: Song) -> [String: Any] {
        var value: [String: Any] = [
            "id": song.id.rawValue,
            "title": song.title,
            "artist": song.artistName,
            "album": song.albumTitle ?? "",
            "duration": song.duration ?? 0,
            "playable": song.playParameters != nil,
        ]
        if let number = song.trackNumber { value["track"] = number }
        if let number = song.discNumber { value["disc"] = number }
        // Only relationships MusicKit already loaded are read: fetching them would cost one
        // more request per song.
        if let id = song.albums?.first?.id { value["albumId"] = id.rawValue }
        if let id = song.artists?.first?.id { value["artistId"] = id.rawValue }
        return value
    }

    // A playlist or album can also hold music videos, which Rune does not list.
    private func track(_ track: Track) -> [String: Any]? {
        if case .song(let value) = track { return song(value) }
        return nil
    }

    private func album(_ album: Album) -> [String: Any] {
        ["id": album.id.rawValue, "title": album.title, "artist": album.artistName, "songCount": album.trackCount]
    }

    private func artist(_ artist: Artist) -> [String: Any] {
        ["id": artist.id.rawValue, "name": artist.name]
    }

    private func playlist(_ playlist: Playlist) -> [String: Any] {
        ["id": playlist.id.rawValue, "name": playlist.name]
    }
}

/// What the player is doing, as every player op reports it.
@available(macOS 14.0, iOS 16.0, *)
private struct JukeboxSnapshot: Sendable {
    let playing: Bool
    let position: TimeInterval
    let currentIndex: Int?
    let ids: [String]

    var json: [String: Any] {
        var value: [String: Any] = ["playing": playing, "position": position, "ids": ids]
        if let currentIndex { value["currentIndex"] = currentIndex }
        return value
    }
}

/// Apple's player driven as a Subsonic jukebox. `songs` is the queue Rune sees; the player's own
/// queue is rebuilt from it whenever Rune changes anything but the end of it.
@available(macOS 14.0, iOS 16.0, *)
private actor AppleMusicJukebox {
    static let shared = AppleMusicJukebox()

    private var songs: [Song] = []

    // The player drops a song it has no play information for, which would shift every later
    // entry. It is given only the others, and these are their places in `songs`.
    private var playable: [Int] { songs.indices.filter { songs[$0].playParameters != nil } }

    private func playerQueue(startingAt index: Int) throws -> ApplicationMusicPlayer.Queue {
        guard songs[index].playParameters != nil else {
            throw AppleMusicFailure(kind: "unplayable", detail: "Apple Music cannot play this song")
        }
        return ApplicationMusicPlayer.Queue(for: playable.map { songs[$0] }, startingAt: songs[index])
    }

    private func idleQueue() -> ApplicationMusicPlayer.Queue {
        guard let first = playable.first else { return [] }
        return ApplicationMusicPlayer.Queue(for: playable.map { songs[$0] }, startingAt: songs[first])
    }

    // Apple's player reports a song it cannot load only as a failure to prepare.
    private func play() async throws {
        do {
            try await player.play()
        } catch {
            throw AppleMusicFailure(kind: "unplayable", detail: "Apple Music could not play this song: \(error.localizedDescription)")
        }
    }

    // Not isolated to the actor: the player's async calls run off the actor, and a reference
    // obtained here may be handed to them. The actor still orders every use of it.
    private nonisolated var player: ApplicationMusicPlayer { ApplicationMusicPlayer.shared }

    private var isPlaying: Bool { player.state.playbackStatus == .playing }

    /// Replaces the queue without starting it, as a Subsonic jukebox `set` does.
    func set(_ songs: [Song]) -> JukeboxSnapshot {
        player.stop()
        self.songs = songs
        player.queue = idleQueue()
        return snapshot()
    }

    /// Appends to the queue. The player's queue is extended in place so the current song plays on.
    func add(_ added: [Song]) async throws -> JukeboxSnapshot {
        guard !added.isEmpty else { return snapshot() }
        guard !songs.isEmpty else { return set(added) }
        try await player.queue.insert(added.filter { $0.playParameters != nil }, position: .tail)
        songs.append(contentsOf: added)
        return snapshot()
    }

    func clear() -> JukeboxSnapshot {
        player.stop()
        songs = []
        player.queue = []
        return snapshot()
    }

    /// Removes one entry. An index past the end changes nothing, as in a Subsonic jukebox.
    func remove(at index: Int) async throws -> JukeboxSnapshot {
        guard songs.indices.contains(index) else { return snapshot() }
        let current = currentIndex()
        let playing = isPlaying
        let position = player.playbackTime
        songs.remove(at: index)
        guard !songs.isEmpty else { return clear() }
        guard let current else {
            player.queue = idleQueue()
            return snapshot()
        }
        // The removed song is still in the player's queue, so the queue is rebuilt from `songs`.
        // Rebuilding restarts the current entry: its position is restored and playback resumed,
        // but the listener may hear a short gap.
        if index != current {
            try await start(at: index < current ? current - 1 : current, offset: position, play: playing)
        } else if current < songs.count {
            try await start(at: current, offset: 0, play: playing)
        } else {
            // The last song was playing and is gone; like the end of the queue, nothing plays.
            player.stop()
            player.queue = idleQueue()
        }
        return snapshot()
    }

    /// Moves to `index` at `offset` seconds and keeps playing only if it was playing. Within the
    /// current entry only the position changes, so a seek does not restart the song.
    func skip(to index: Int, offset: TimeInterval) async throws -> JukeboxSnapshot {
        guard songs.indices.contains(index) else {
            throw AppleMusicFailure(kind: "invalid", detail: "no queue entry \(index)")
        }
        if index == currentIndex() {
            player.playbackTime = offset
        } else {
            try await start(at: index, offset: offset, play: isPlaying)
        }
        return snapshot()
    }

    /// Sets how the player moves through the queue. Shuffling is done by Apple's player, which
    /// then picks the next song itself.
    func setMode(_ mode: String) throws -> JukeboxSnapshot {
        switch mode {
        case "sequential":
            player.state.repeatMode = MusicPlayer.RepeatMode.none
            player.state.shuffleMode = .off
        case "repeatOne":
            player.state.repeatMode = .one
            player.state.shuffleMode = .off
        case "repeatAll":
            player.state.repeatMode = .all
            player.state.shuffleMode = .off
        case "shuffle":
            player.state.repeatMode = MusicPlayer.RepeatMode.none
            player.state.shuffleMode = .songs
        default:
            throw AppleMusicFailure(kind: "invalid", detail: "unknown playback mode \(mode)")
        }
        return snapshot()
    }

    func start() async throws -> JukeboxSnapshot {
        if !songs.isEmpty {
            try await play()
        }
        return snapshot()
    }

    func stop() -> JukeboxSnapshot {
        player.pause()
        return snapshot()
    }

    func snapshot() -> JukeboxSnapshot {
        JukeboxSnapshot(
            playing: isPlaying,
            position: player.playbackTime,
            currentIndex: currentIndex(),
            ids: songs.map { $0.id.rawValue }
        )
    }

    private func start(at index: Int, offset: TimeInterval, play: Bool) async throws {
        player.queue = try playerQueue(startingAt: index)
        if offset > 0 {
            // The new entry is prepared first: a position set before it is loaded may not apply.
            try await player.prepareToPlay()
            player.playbackTime = offset
        }
        if play {
            try await self.play()
        }
    }

    // The player's entries are the playable songs in order. The current one is found by its place
    // among them: the song an entry names carries its catalogue id, not the library id.
    private func currentIndex() -> Int? {
        guard let current = player.queue.currentEntry else { return nil }
        guard let place = player.queue.entries.firstIndex(where: { $0.id == current.id }) else { return nil }
        let places = playable
        return places.indices.contains(place) ? places[place] : nil
    }
}
