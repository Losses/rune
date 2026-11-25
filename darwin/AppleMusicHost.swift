// Compiled by both the macOS and the iOS runner.
import Combine
import Foundation
import MusicKit
import Network

#if os(macOS)
  import AppKit
  @preconcurrency import FlutterMacOS
#elseif os(iOS)
  import UIKit
  @preconcurrency import Flutter
#endif

/// Plays Apple Music for the hub's hosted player with MusicKit.
///
/// Dart only forwards. Each batch of commands arrives as JSON text (`batch`), and every event
/// goes back as JSON text through an `event` call. `authorize` and `status` are separate calls:
/// Dart attaches with `status`, whose answer is the host's `hostReady`. The player needs macOS
/// 14 or iOS 16.1; on older systems the host reports that it cannot play.
@MainActor
final class AppleMusicHost {
  static let shared = AppleMusicHost()

  private var channel: FlutterMethodChannel?
  /// The `MusicKitPlayback`, which only exists on macOS 14, iOS 16.1 and later.
  private var playback: AnyObject?

  /// What the latest `hostReady` was based on, so that only a change is reported again.
  private var lastAuthorized: Bool?
  private var lastCanPlay: Bool?
  private var activeObserver: NSObjectProtocol?
  private var subscriptionWatch: Task<Void, Never>?
  /// The pending or running question of whether a host that cannot play can play again.
  private var canPlayCheck: Task<Void, Never>?
  /// The pending or running report of the network, and the last one sent.
  private var reachTask: Task<Void, Never>?
  private var lastReach: String?

  private init() {}

  /// Creates the channel on the engine's messenger. Called once the Flutter view controller exists.
  func attach(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(
      name: "ci.not.rune/apple_music", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] (call, result) in
      // Flutter delivers channel calls on the main thread.
      MainActor.assumeIsolated {
        guard let self else {
          result(FlutterMethodNotImplemented)
          return
        }
        self.handle(call, result)
      }
    }
    self.channel = channel
    lastAuthorized = Self.isAuthorized
    NetworkReach.shared.start { [weak self] in
      Task { @MainActor in
        self?.reportReach(after: Self.reachDebounce)
        // A subscription that could not be read without a network can be read again.
        if NetworkReach.shared.isOnline {
          self?.recheckCanPlay(after: Self.reachDebounce)
        }
      }
    }

    // Access can be changed in the system settings while Rune runs. A reattach replaces the
    // observer rather than adding another.
    if let activeObserver {
      NotificationCenter.default.removeObserver(activeObserver)
    }
    activeObserver = NotificationCenter.default.addObserver(
      forName: Self.becameActive, object: nil, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated {
        guard let self else { return }
        // What is downloaded may have changed while Rune was in the background.
        self.reportReach(after: 0)
        guard Self.isAuthorized != self.lastAuthorized else {
          self.recheckCanPlay(after: 0)
          return
        }
        self.lastAuthorized = Self.isAuthorized
        Task { await self.restart() }
      }
    }
  }

  #if os(macOS)
    private static let becameActive = NSApplication.didBecomeActiveNotification
  #elseif os(iOS)
    private static let becameActive = UIApplication.didBecomeActiveNotification
  #endif

  fileprivate static var isAuthorized: Bool {
    if #available(macOS 14, iOS 16.1, *) {
      return MusicAuthorization.currentStatus == .authorized
    }
    return false
  }

  private func handle(_ call: FlutterMethodCall, _ result: @escaping FlutterResult) {
    switch call.method {
    case "batch":
      guard let json = call.arguments as? String else {
        result(FlutterError(code: "bad_arguments", message: "A batch is JSON text", details: nil))
        return
      }
      if #available(macOS 14, iOS 16.1, *) {
        player().submit(json)
      }
      result(nil)
    case "status":
      Task {
        let canPlay = await self.currentCanPlay()
        // Dart attaches with this call, and the player takes the answer for a host that just
        // started. Whatever is still held from before goes in the same turn as the answer.
        self.dropQueue()
        result(self.readyEvent(canPlay: canPlay))
        // The hub learns the network state of a host that just attached.
        self.reportReach(after: 0)
      }
    case "authorize":
      let interactive = (call.arguments as? [String: Any])?["interactive"] as? Bool ?? false
      Task {
        let reply = await Self.authorize(interactive: interactive)
        // A change of access restarts the host. Without one the host keeps what it holds: a
        // `hostReady` would make the player hand its track over again.
        if Self.isAuthorized != self.lastAuthorized {
          self.lastAuthorized = Self.isAuthorized
          await self.restart()
        }
        result(reply)
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  @available(macOS 14, iOS 16.1, *)
  private func player() -> MusicKitPlayback {
    if let existing = playback as? MusicKitPlayback {
      return existing
    }
    let created = MusicKitPlayback(
      send: { [weak self] json in
        self?.channel?.invokeMethod("event", arguments: json)
      },
      lapsed: { [weak self] in
        // The playback dropped its queue already, in this same turn.
        guard let self else { return }
        self.channel?.invokeMethod("event", arguments: self.readyEvent(canPlay: false))
      })
    playback = created
    return created
  }

  /// Reports the host as started again. It drops its queue and says so in the same turn, so that
  /// no batch comes in between, since the player then hands the current track over again.
  private func restart(canPlay known: Bool? = nil) async {
    let canPlay: Bool
    if let known {
      canPlay = known
    } else {
      canPlay = await currentCanPlay()
    }
    dropQueue()
    channel?.invokeMethod("event", arguments: readyEvent(canPlay: canPlay))
  }

  private func dropQueue() {
    if #available(macOS 14, iOS 16.1, *) {
      (playback as? MusicKitPlayback)?.drop()
    }
  }

  /// Whether the catalogue can be played. It never asks the user for anything.
  private func currentCanPlay() async -> Bool {
    guard #available(macOS 14, iOS 16.1, *), MusicAuthorization.currentStatus == .authorized else {
      return false
    }
    // Asking MusicKit about the subscription before access is granted leaves it in a cached
    // error state, so only ask once authorized.
    return (try? await MusicSubscription.current.canPlayCatalogContent) ?? false
  }

  /// Asks again whether the catalogue can be played, after `delay` nanoseconds, when the host
  /// last said that it cannot, or never said. The answer may have been an error that passed, as
  /// without a network, and no change of the subscription follows one. Only a host that can
  /// play again starts again, since a restart takes the queue from one that plays. A question
  /// that is still waiting is replaced.
  private func recheckCanPlay(after delay: UInt64) {
    guard Self.isAuthorized, lastCanPlay != true else { return }
    canPlayCheck?.cancel()
    canPlayCheck = Task { [weak self] in
      if delay > 0 {
        try? await Task.sleep(nanoseconds: delay)
      }
      guard !Task.isCancelled, let self else { return }
      let canPlay = await self.currentCanPlay()
      guard !Task.isCancelled, canPlay, Self.isAuthorized, self.lastCanPlay != true else {
        return
      }
      await self.restart(canPlay: true)
    }
  }

  /// How long a change of the network settles before it is reported, in nanoseconds.
  private static let reachDebounce: UInt64 = 1_000_000_000
  /// How often the downloaded songs are read again while offline, as a download may finish, in
  /// nanoseconds.
  private static let offlineRefresh: UInt64 = 60_000_000_000

  /// Reports whether the device is online to the hub, and while it is offline, which library
  /// songs are stored on it, after `delay` nanoseconds. A report that is still waiting is
  /// replaced. While offline the report repeats, and is only sent again when it changed.
  private func reportReach(after delay: UInt64, onlyIfChanged: Bool = false) {
    reachTask?.cancel()
    reachTask = Task { [weak self] in
      if delay > 0 {
        try? await Task.sleep(nanoseconds: delay)
      }
      guard !Task.isCancelled else { return }
      let online = NetworkReach.shared.isOnline
      var event: [String: Any] = ["type": "networkState", "host": "appleMusic", "online": online]
      if !online {
        // A read that failed is tried again later rather than taken for an empty library.
        if let downloaded = await Self.downloadedSongIDs() {
          event["downloaded"] = downloaded
        } else {
          self?.reportReach(after: Self.offlineRefresh, onlyIfChanged: true)
          return
        }
      }
      guard !Task.isCancelled, let self else { return }
      let json = encode(event)
      if !(onlyIfChanged && json == self.lastReach) {
        self.lastReach = json
        self.channel?.invokeMethod("event", arguments: json)
      }
      if !online {
        self.reportReach(after: Self.offlineRefresh, onlyIfChanged: true)
      }
    }
  }

  /// The ids of the library songs stored on this device, sorted so that equal sets read equal;
  /// nil when they could not be read.
  nonisolated private static func downloadedSongIDs() async -> [String]? {
    guard #available(macOS 14, iOS 16.1, *), MusicAuthorization.currentStatus == .authorized else {
      return nil
    }
    let pageSize = 500
    var ids: [String] = []
    var offset = 0
    while true {
      var request = MusicLibraryRequest<Song>()
      request.includeOnlyDownloadedContent = true
      request.limit = pageSize
      request.offset = offset
      do {
        let page = try await request.response().items
        ids.append(contentsOf: page.map { $0.id.rawValue })
        guard page.count == pageSize else { return ids.sorted() }
        offset += pageSize
      } catch {
        NSLog("%@", "Apple Music host: the downloaded songs could not be read: \(error)")
        return nil
      }
    }
  }

  /// The `hostReady` event for the current authorization, which it records.
  private func readyEvent(canPlay: Bool) -> String {
    // Before macOS 14 or iOS 16.1 there is nothing to sign in to, so Rune is not asked to.
    var authorized = true
    if #available(macOS 14, iOS 16.1, *) {
      authorized = MusicAuthorization.currentStatus == .authorized
      if authorized {
        watchSubscription()
      }
    }
    lastAuthorized = Self.isAuthorized
    lastCanPlay = canPlay
    return encode([
      "type": "hostReady",
      "host": "appleMusic",
      "authorized": authorized,
      "canPlay": canPlay,
    ])
  }

  /// A subscription that starts or ends changes what can be played.
  @available(macOS 14, iOS 16.1, *)
  private func watchSubscription() {
    guard subscriptionWatch == nil else { return }
    subscriptionWatch = Task { [weak self] in
      for await subscription in MusicSubscription.subscriptionUpdates {
        guard let self else { return }
        let canPlay = subscription.canPlayCatalogContent
        if MusicAuthorization.currentStatus == .authorized, canPlay != self.lastCanPlay {
          await self.restart(canPlay: canPlay)
        }
      }
    }
  }

  /// Authorizes and fetches the tokens the hub needs. The system's consent dialog only shows when
  /// `interactive` is set. Without tokens, `reason` says why: `unsupported` before macOS 14 or iOS 16.1,
  /// `refused` without access, `account` when the device's account cannot use Apple Music,
  /// `privacy` when the user has yet to accept Apple Music's privacy notice, and `noToken` when
  /// access was given but the tokens could not be had otherwise.
  private static func authorize(interactive: Bool) async -> [String: Any] {
    guard #available(macOS 14, iOS 16.1, *) else { return ["authorized": false, "reason": "unsupported"] }
    if interactive, MusicAuthorization.currentStatus != .authorized {
      _ = await MusicAuthorization.request()
    }
    guard MusicAuthorization.currentStatus == .authorized else {
      return ["authorized": false, "reason": "refused"]
    }
    var reply: [String: Any] = ["authorized": true]
    // Never log the tokens.
    var step = "developer"
    do {
      // Asking before authorization puts MusicKit into a cached XPC error state.
      let provider = DefaultMusicTokenProvider()
      let developerToken = try await provider.developerToken(options: [])
      step = "user"
      let userToken = try await provider.userToken(for: developerToken, options: [])
      reply["token"] = userToken
      reply["developerToken"] = developerToken
    } catch {
      // The tokens are left out; Dart treats that as not signed in.
      reply["reason"] = await tokenFailureReason(error)
      NSLog(
        "%@", "Apple Music host: fetching the \(step) token failed: \(describeFailure(error))")
      // The subscription flags are only for the log, so the reply does not wait on MusicKit.
      Task.detached { await logWhyTokensFailed() }
    }
    return reply
  }

  /// How long the reason for a failed user token waits for MusicKit's view of the subscription.
  /// The user waits for the answer.
  @available(macOS 14, iOS 16.1, *)
  private static var subscriptionWait: Duration { .seconds(2) }

  /// Why the tokens could not be had, as the user can act on it. A user token also fails without
  /// a network, so only an account that MusicKit says cannot play the catalogue counts as the
  /// account's fault.
  @available(macOS 14, iOS 16.1, *)
  private static func tokenFailureReason(_ error: Error) async -> String {
    switch error as? MusicTokenRequestError {
    case .privacyAcknowledgementRequired:
      return "privacy"
    case .userNotSignedIn:
      return "account"
    case .userTokenRequestFailed:
      let canPlay = try? await bounded(until: .now + subscriptionWait) {
        try await MusicSubscription.current.canPlayCatalogContent
      }
      return canPlay == false ? "account" : "noToken"
    default:
      return "noToken"
    }
  }

  /// Logs MusicKit's view of this device's Apple Music account when the tokens could not be had.
  /// A user token fails when the account the device is signed in with cannot use Apple Music, and
  /// these flags tell that apart from a fault in Rune. Nothing logged is a token.
  @available(macOS 14, iOS 16.1, *)
  nonisolated private static func logWhyTokensFailed() async {
    do {
      let subscription = try await MusicSubscription.current
      NSLog(
        "%@",
        "Apple Music host: subscription canPlayCatalogContent=\(subscription.canPlayCatalogContent) "
          + "canBecomeSubscriber=\(subscription.canBecomeSubscriber) "
          + "hasCloudLibraryEnabled=\(subscription.hasCloudLibraryEnabled)")
    } catch {
      NSLog("%@", "Apple Music host: reading the subscription failed: \(describeFailure(error))")
    }
  }
}

/// What a log may say about an error: a MusicKit token error's case, or else only the type and
/// the NSError domain and code, with those of the underlying error. `userInfo` is never printed.
nonisolated private func describeFailure(_ error: Error) -> String {
  if #available(macOS 14, iOS 16.1, *), let token = error as? MusicTokenRequestError {
    return "MusicTokenRequestError.\(token.rawValue)"
  }
  let nsError = error as NSError
  var text = "\(type(of: error)) \(nsError.domain) \(nsError.code)"
  if let underlying = nsError.userInfo[NSUnderlyingErrorKey] as? NSError {
    text += " (underlying \(underlying.domain) \(underlying.code))"
  }
  return text
}

private func encode(_ object: [String: Any]) -> String {
  guard let data = try? JSONSerialization.data(withJSONObject: object),
    let text = String(data: data, encoding: .utf8)
  else {
    return "{}"
  }
  return text
}

/// One action of a batch. Which fields are set depends on `action`.
private struct WireAction: Decodable, Sendable {
  let action: String
  let ids: [String]?
  let index: Int?
  let offsetMs: Double?
  let gain: Double?
}

private struct WireBatch: Decodable, Sendable {
  let revision: UInt64
  let actions: [WireAction]
}

/// What the library in Apple Music answers about library songs: the catalogue song of each, by
/// its play parameters or else by its included catalogue relationship.
nonisolated private struct LibrarySongsAnswer: Decodable, Sendable {
  nonisolated struct Item: Decodable, Sendable {
    nonisolated struct Attributes: Decodable, Sendable {
      nonisolated struct PlayParams: Decodable, Sendable {
        let catalogId: String?
      }
      let playParams: PlayParams?
    }
    nonisolated struct Relationships: Decodable, Sendable {
      nonisolated struct Catalogue: Decodable, Sendable {
        nonisolated struct Entry: Decodable, Sendable {
          let id: String
        }
        let data: [Entry]
      }
      let catalog: Catalogue?
    }
    let id: String
    let attributes: Attributes?
    let relationships: Relationships?

    var catalogueID: String? {
      attributes?.playParams?.catalogId ?? relationships?.catalog?.data.first?.id
    }
  }
  let data: [Item]
}

/// Hands one result to whoever waits, once: the work, the timeout or a cancel, whichever is first.
nonisolated private final class Gate<T: Sendable>: @unchecked Sendable {
  private let lock = NSLock()
  private var result: Result<T, Error>?
  private var continuation: CheckedContinuation<T, Error>?

  func resolve(_ value: Result<T, Error>) {
    lock.lock()
    guard result == nil else {
      lock.unlock()
      return
    }
    result = value
    let waiting = continuation
    continuation = nil
    lock.unlock()
    waiting?.resume(with: value)
  }

  func wait() async throws -> T {
    try await withCheckedThrowingContinuation { (waiting: CheckedContinuation<T, Error>) in
      lock.lock()
      if let result {
        lock.unlock()
        waiting.resume(with: result)
      } else {
        continuation = waiting
        lock.unlock()
      }
    }
  }
}

nonisolated private struct TimedOut: Error {}

nonisolated private struct PlaybackStopped: Error {}

/// How the host leaves out a time that the process did not run, as in an app that the system
/// suspended.
@available(macOS 14, iOS 16.1, *)
nonisolated private enum Freeze {
  /// A wait or a periodic check that ends this much later than it should means that the process
  /// did not run meanwhile. The player takes the same threshold.
  static let gap: Duration = .seconds(5)
  /// How often a deadline is checked against that.
  static let step: Duration = .seconds(1)
}

/// Runs `work` until `deadline`, or until the task that waits for it is cancelled, as a newer
/// pick does. The work keeps running if MusicKit ignores the cancel, but nobody waits for it any
/// more. A time that the process did not run moves the deadline on: the work could not be
/// answered meanwhile, so it does not time out for it.
@available(macOS 14, iOS 16.1, *)
private func bounded<T: Sendable>(
  until deadline: ContinuousClock.Instant,
  _ work: @escaping @MainActor @Sendable () async throws -> T
) async throws -> T {
  let gate = Gate<T>()
  let worker = Task {
    do {
      gate.resolve(.success(try await work()))
    } catch {
      gate.resolve(.failure(error))
    }
  }
  let watchdog = Task {
    var deadline = deadline
    var last = ContinuousClock.now
    while last < deadline {
      let step = min(Freeze.step, deadline - last)
      do {
        try await Task.sleep(for: step)
      } catch {
        return
      }
      let woke = ContinuousClock.now
      let frozen = woke - last - step
      if frozen > Freeze.gap {
        deadline += frozen
      }
      last = woke
    }
    gate.resolve(.failure(TimedOut()))
    worker.cancel()
  }
  defer { watchdog.cancel() }
  return try await withTaskCancellationHandler {
    try await gate.wait()
  } onCancel: {
    gate.resolve(.failure(CancellationError()))
    worker.cancel()
  }
}

private enum Failure: String {
  case unplayable
  case notAuthorized
  case network
}

/// An entry of the host's queue: the id the player gave, and once found, its song and whether
/// MusicKit's queue holds it.
@available(macOS 14, iOS 16.1, *)
private final class Entry {
  let id: String
  var song: Song?
  /// Whether the entry went into MusicKit's queue, by a new queue or an insert.
  var queued = false
  /// The id of the MusicKit queue entry that plays it. MusicKit may not give one at once, or give
  /// the entry a new one, and the song then tells which queue entry it is.
  var queueEntryID: String?
  var failure: Failure?
  var lookup: Task<Void, Never>?

  init(id: String) {
    self.id = id
  }

  var failed: Bool { failure != nil }
}

#if os(iOS)
  /// Background time for an app that is not active while MusicKit takes over from Rune's own
  /// audio, or while the tracks after the current one have yet to reach MusicKit. Without audio
  /// of its own the app may be suspended before they do, and MusicKit then plays what it holds
  /// and stops.
  @available(iOS 16.1, *)
  private final class HandOverHold {
    /// How long a hold lasts at most, within the system's limit for background time.
    private static let limit: Duration = .seconds(25)
    /// How long after the latest batch a hold still lasts, as the player sends the tracks after
    /// the current one one after another.
    private static let settle: Duration = .seconds(3)

    private var task: UIBackgroundTaskIdentifier = .invalid
    private var began = ContinuousClock.now
    private var lastBatch = ContinuousClock.now

    /// A batch came.
    func noteBatch() {
      lastBatch = .now
    }

    /// Holds the app awake, unless it does already.
    func take() {
      guard task == .invalid else { return }
      began = .now
      task = UIApplication.shared.beginBackgroundTask(withName: "Apple Music hand-over") {
        [weak self] in
        MainActor.assumeIsolated { self?.release() }
      }
    }

    /// Lets the app go once the work is `settled` and no batch came for a while, or once the
    /// hold lasted as long as it may.
    func releaseIfDone(settled: Bool) {
      guard task != .invalid else { return }
      let now = ContinuousClock.now
      if (settled && now - lastBatch >= Self.settle) || now - began >= Self.limit {
        release()
      }
    }

    /// Lets the app be suspended again.
    func release() {
      guard task != .invalid else { return }
      UIApplication.shared.endBackgroundTask(task)
      task = .invalid
    }
  }
#endif

/// Songs looked up together: the lookups asked for within a moment of each other go out as one
/// request to the catalogue and one to the library, so that a run of tracks, which the player
/// hands over one after another, costs a few requests rather than one for each track. `i.…` is a
/// library song, `catalog:<id>` a catalogue song.
@available(macOS 14, iOS 16.1, *)
private final class SongLookups {
  /// How long a lookup waits for others to go out with it.
  private static let gathering: Duration = .milliseconds(50)
  /// How many songs one request asks for at most.
  nonisolated private static let perRequest = 25

  private var waiting: [String: [CheckedContinuation<Song?, Error>]] = [:]
  private var flush: Task<Void, Never>?

  /// The song of `id`, or none when there is no such song.
  func song(_ id: String) async throws -> Song? {
    try await withCheckedThrowingContinuation { (answer: CheckedContinuation<Song?, Error>) in
      waiting[id, default: []].append(answer)
      guard flush == nil else { return }
      flush = Task {
        try? await Task.sleep(for: Self.gathering)
        await self.answer()
      }
    }
  }

  /// Answers every lookup gathered so far.
  private func answer() async {
    let asked = waiting
    waiting = [:]
    flush = nil
    let found = await Self.songs(Array(asked.keys))
    for (id, answers) in asked {
      let result = found[id] ?? .success(nil)
      for answer in answers {
        answer.resume(with: result)
      }
    }
  }

  /// Looks up the songs of `ids`, the catalogue's and the library's apart.
  nonisolated private static func songs(_ ids: [String]) async -> [String: Result<Song?, Error>] {
    var catalogue: [(MusicItemID, String)] = []
    var library: [(MusicItemID, String)] = []
    for id in ids {
      if let catalogID = id.removingPrefix("catalog:") {
        catalogue.append((MusicItemID(catalogID), id))
      } else {
        library.append((MusicItemID(id), id))
      }
    }
    var results: [String: Result<Song?, Error>] = [:]
    await look(up: catalogue, into: &results) { itemIDs in
      var request = MusicCatalogResourceRequest<Song>(matching: \.id, memberOf: itemIDs)
      request.limit = itemIDs.count
      return try await request.response().items
    }
    await look(up: library, into: &results) { itemIDs in
      var request = MusicLibraryRequest<Song>()
      request.filter(matching: \.id, memberOf: itemIDs)
      request.limit = itemIDs.count
      return try await request.response().items
    }
    // A song added to the library elsewhere may not have reached this device's library yet. The
    // library in Apple Music names its catalogue song, which plays in its place.
    let missing = library.filter { _, id in
      switch results[id] {
      case .some(.success(.none)), .none: return true
      default: return false
      }
    }
    // Offline the catalogue cannot be asked, and a song that is not downloaded cannot play.
    guard !missing.isEmpty, NetworkReach.shared.isOnline else { return results }
    let catalogueIDs = await catalogueIDs(of: missing.map(\.0))
    let standIns = missing.compactMap { libraryID, id in catalogueIDs[libraryID].map { ($0, id) } }
    guard !standIns.isEmpty else { return results }
    NSLog(
      "%@",
      "Apple Music host: \(standIns.count) of \(missing.count) songs missing from this device's library play from the catalogue"
    )
    await look(up: standIns, into: &results) { itemIDs in
      var request = MusicCatalogResourceRequest<Song>(matching: \.id, memberOf: itemIDs)
      request.limit = itemIDs.count
      return try await request.response().items
    }
    return results
  }

  /// The catalogue songs of library songs, by library song, as the library in Apple Music names
  /// them. A song it names none for, as an upload, is left out, and so is every song of a request
  /// that fails.
  nonisolated private static func catalogueIDs(of libraryIDs: [MusicItemID]) async
    -> [MusicItemID: MusicItemID]
  {
    var found: [MusicItemID: MusicItemID] = [:]
    for start in stride(from: 0, to: libraryIDs.count, by: perRequest) {
      let chunk = libraryIDs[start..<min(start + perRequest, libraryIDs.count)]
      var components = URLComponents()
      components.scheme = "https"
      components.host = "api.music.apple.com"
      components.path = "/v1/me/library/songs"
      components.queryItems = [
        URLQueryItem(name: "ids", value: chunk.map(\.rawValue).joined(separator: ",")),
        URLQueryItem(name: "include", value: "catalog"),
      ]
      guard let url = components.url else { continue }
      do {
        let response = try await MusicDataRequest(urlRequest: URLRequest(url: url)).response()
        let answer = try JSONDecoder().decode(LibrarySongsAnswer.self, from: response.data)
        for song in answer.data {
          if let catalogueID = song.catalogueID {
            found[MusicItemID(song.id)] = MusicItemID(catalogueID)
          }
        }
      } catch {
        NSLog("%@", "Apple Music host: the library named no catalogue songs: \(error)")
      }
    }
    return found
  }

  /// Asks `request` for the songs of `asked`, a few at a time, and records each one under the id
  /// it was asked for. A song that the answer does not hold under the item id asked for, as an
  /// answer may name a song by another id, is asked for alone, and the first song that comes is
  /// it, as a lookup of one song always took it. A request that fails fails each song it asked
  /// for.
  nonisolated private static func look(
    up asked: [(MusicItemID, String)],
    into results: inout [String: Result<Song?, Error>],
    with request: ([MusicItemID]) async throws -> MusicItemCollection<Song>
  ) async {
    for start in stride(from: 0, to: asked.count, by: perRequest) {
      let chunk = asked[start..<min(start + perRequest, asked.count)]
      let songs: MusicItemCollection<Song>
      do {
        songs = try await request(chunk.map(\.0))
      } catch {
        for (_, id) in chunk {
          results[id] = .failure(error)
        }
        continue
      }
      for (itemID, id) in chunk {
        if let song = songs.first(where: { $0.id == itemID }) {
          results[id] = .success(song)
          continue
        }
        do {
          results[id] = .success(try await request([itemID]).first)
        } catch {
          results[id] = .failure(error)
        }
      }
    }
  }
}

/// The hosted player's protocol on top of `ApplicationMusicPlayer`.
///
/// The host's queue holds the entries the player gave since its latest `set`, and indexes count
/// from 0 there. MusicKit's queue gets an entry once its song is found, by an insert at its
/// tail that leaves what plays alone. A host entry knows its MusicKit queue entry by that entry's
/// id, or else by its song. MusicKit's current entry changing to a later entry's is the host
/// moving on; the last entry playing out with nothing playable after it is the end. The entries
/// that played go out of MusicKit's queue, so that the system's controls only ever go back to the
/// start of the current entry.
///
/// MusicKit plays on through its queue while the host cannot run, as in an app that the system
/// suspended. Once the host runs again it takes what MusicKit did meanwhile as it stands: a move
/// by several entries, an end, or a pause from the system's controls. No deadline counts the time
/// the host did not run, and nothing that played is reported as failed. On iOS a hand-over that
/// comes while the app is not active holds the app awake until MusicKit plays, as the app may
/// otherwise be suspended between the end of its own audio and the start of MusicKit's.
///
/// Batches and queue edits run one after another in the order they arrive. Every event carries
/// the revision of the latest batch that was applied. A batch with `set` or `clear` makes every
/// batch, lookup and insert queued before it stale, and cancels the MusicKit calls they wait
/// for, so the newest pick wins and nothing older touches its queue.
@available(macOS 14, iOS 16.1, *)
@MainActor
final class MusicKitPlayback {
  nonisolated(unsafe) private let player = ApplicationMusicPlayer.shared
  private let send: (String) -> Void
  /// The subscription lapsed, and the queue was dropped: the host can play nothing any more.
  private let lapsed: () -> Void

  /// How long a lookup may take outside a batch.
  private static let timeout: Duration = .seconds(20)
  /// How long an insert into MusicKit's queue may hold up the batches queued behind it.
  private static let insertTimeout: Duration = .seconds(5)
  /// How often an insert looks whether MusicKit has confirmed the songs given before it.
  private static let settlePoll: Duration = .milliseconds(50)
  /// The player counts a track as unreachable when its host does not report within 10 s of
  /// being told to play it. A batch gives up on MusicKit sooner, so it reports that itself.
  private static let batchBudget: Duration = .seconds(9)
  /// How long an entry that was told to play may stay silent before it fails.
  private static let startLimit: Duration = .seconds(8)
  /// How often a status goes out while a track plays.
  private static let reportEvery: Duration = .seconds(2)
  /// How short of its end, in seconds, an entry counts as played out, as MusicKit may stop just
  /// before it.
  private static let endTolerance: Double = 1
  /// How close to its start, in seconds, an entry stands that MusicKit took back there as its
  /// queue ran out.
  private static let startTolerance: Double = 1

  /// Songs resolved earlier, so a replay does not ask again.
  private var songs: [String: Song] = [:]

  private var tail: Task<Void, Never>?
  /// The batches and queue edits that are queued or running, so a newer pick can cancel them.
  private var running: [Int: Task<Void, Never>] = [:]
  private var nextJob = 0
  /// Counts the picks: `set`, `clear`, and the host dropping its queue. Work queued for an
  /// older pick is stale.
  private var epoch = 0
  private var revision: UInt64 = 0
  /// Events are held back while a batch is being applied; the batch answers once it is done.
  private var applying = false
  /// Whether a lapsed subscription was reported since the latest `set`.
  private var lapseReported = false

  private var entries: [Entry] = []
  private var current: Int?
  /// Whether the last entry finished with nothing playable after it.
  private var ended = false
  /// Whether Rune wants the current entry to play.
  private var wantPlaying = false {
    didSet {
      #if os(macOS)
        if wantPlaying != oldValue { holdActivity(wantPlaying) }
      #endif
    }
  }
  #if os(macOS)
    /// The activity that keeps App Nap from slowing the periodic check while Rune wants the
    /// current entry to play: the player takes a host that stops reporting for a failed one.
    private var playingActivity: NSObjectProtocol?
  #endif
  /// When Rune last paused, since MusicKit may still say playing just after.
  private var stoppedAt = ContinuousClock.now
  /// When the current entry was told to play and has not been heard playing since.
  private var startedAt: ContinuousClock.Instant?
  /// Whether the current entry was heard playing since it became current or was last started.
  private var sawPlaying = false
  /// The furthest position seen, since MusicKit may reset the time when an entry ends.
  private var furthest: Double = 0
  /// When the current entry was first seen played out while an entry follows it.
  private var stalledSince: ContinuousClock.Instant?
  /// Whether the host moves on to the next entry itself, as MusicKit did not.
  private var advancing = false
  /// The pick whose next insert waits for MusicKit to confirm the songs given before it.
  private var settlingEpoch: Int?
  /// The position a `skip` asked for while nothing played, kept for the next `start`, which
  /// may come in a later batch, as after a hand-over of a paused track. A `set`, a `clear`,
  /// another entry becoming current, or playback that begins without a `start` drops it.
  private var pendingOffset: Double?
  /// The MusicKit queue entry last found to map to no track, so that it is logged once.
  private var unmatchedEntryID: String?
  /// When the periodic check last ran, so that a time the host did not run is noticed.
  private var lastTick: ContinuousClock.Instant?
  /// The host did not run for a while, and takes what MusicKit did meanwhile once no batch is
  /// being applied.
  private var resyncDue = false
  /// When the host last noticed that it had not run for a while.
  private var resumedAt: ContinuousClock.Instant?
  private let lookups = SongLookups()
  #if os(iOS)
    private let hold = HandOverHold()
    private var activityObservers: [NSObjectProtocol] = []
  #endif

  private var reportedSignature = ""
  private var reportedAt = ContinuousClock.now
  private var subscriptions: Set<AnyCancellable> = []
  private var queueSubscription: AnyCancellable?
  private var timer: Timer?

  init(send: @escaping (String) -> Void, lapsed: @escaping () -> Void) {
    self.send = send
    self.lapsed = lapsed

    // The player keeps the play order and its modes, so MusicKit plays the queue as it is.
    player.state.repeatMode = MusicPlayer.RepeatMode.none
    player.state.shuffleMode = MusicPlayer.ShuffleMode.off

    player.state.objectWillChange
      .receive(on: DispatchQueue.main)
      .sink { [weak self] _ in self?.check(periodic: false) }
      .store(in: &subscriptions)
    observeQueue()
    #if os(iOS)
      observeActivity()
    #endif
  }

  /// Runs the periodic check while the host holds entries. The common modes keep the timer
  /// running during window resizes, open menus and scrolling.
  private func startTimer() {
    guard timer == nil else { return }
    let timer = Timer(timeInterval: 0.5, repeats: true) { [weak self] _ in
      MainActor.assumeIsolated { self?.check(periodic: true) }
    }
    timer.tolerance = 0.1
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }

  private func stopTimer() {
    timer?.invalidate()
    timer = nil
    lastTick = nil
  }

  #if os(macOS)
    /// Holds the activity while `held`, and ends it otherwise. The system may still sleep when
    /// idle.
    private func holdActivity(_ held: Bool) {
      if held {
        guard playingActivity == nil else { return }
        playingActivity = ProcessInfo.processInfo.beginActivity(
          options: .userInitiatedAllowingIdleSystemSleep,
          reason: "Reporting Apple Music playback to the player")
      } else if let activity = playingActivity {
        ProcessInfo.processInfo.endActivity(activity)
        playingActivity = nil
      }
    }
  #endif

  /// The queue object is replaced when a new queue is set, so its subscription is renewed then.
  private func observeQueue() {
    queueSubscription = player.queue.objectWillChange
      .receive(on: DispatchQueue.main)
      .sink { [weak self] _ in self?.check(periodic: false) }
  }

  private var isPlaying: Bool {
    switch player.state.playbackStatus {
    case .playing, .seekingForward, .seekingBackward: return true
    default: return false
    }
  }

  private var currentEntry: Entry? {
    guard let current, entries.indices.contains(current) else { return nil }
    return entries[current]
  }

  private var nextEntry: Entry? {
    guard let current, entries.indices.contains(current + 1) else { return nil }
    return entries[current + 1]
  }

  private func index(of entry: Entry) -> Int? {
    entries.firstIndex { $0 === entry }
  }

  // MARK: - Batches

  /// Queues one batch, given as JSON text.
  func submit(_ json: String) {
    guard let data = json.data(using: .utf8),
      let batch = try? JSONDecoder().decode(WireBatch.self, from: data)
    else {
      NSLog("%@", "Apple Music host: unreadable batch")
      return
    }
    let deadline = ContinuousClock.now + Self.batchBudget
    if batch.actions.contains(where: { $0.action == "set" || $0.action == "clear" }) {
      supersede()
    }
    #if os(iOS)
      hold.noteBatch()
      // A hand-over has yet to reach MusicKit, and so have a start and a track that the player
      // adds after the current one, while the host holds entries that MusicKit still plays.
      let handsOver = batch.actions.contains { $0.action == "set" }
      let feeds =
        !entries.isEmpty && !ended
        && batch.actions.contains { $0.action == "start" || $0.action == "add" }
      if UIApplication.shared.applicationState != .active, handsOver || feeds {
        hold.take()
      }
    #endif
    let epoch = epoch
    enqueue { [weak self] in await self?.run(batch, epoch: epoch, deadline: deadline) }
  }

  /// Stops and empties the queue, as a host that starts holds nothing.
  func drop() {
    supersede()
    reset()
    #if os(iOS)
      hold.release()
    #endif
  }

  /// Makes everything queued so far stale, and stops the lookups and MusicKit calls it waits for.
  private func supersede() {
    epoch += 1
    for task in running.values {
      task.cancel()
    }
    running.removeAll()
    for entry in entries {
      entry.lookup?.cancel()
    }
  }

  /// Runs `job` after everything queued before it.
  private func enqueue(_ job: @escaping @MainActor () async -> Void) {
    let key = nextJob
    nextJob += 1
    let previous = tail
    let task = Task { [weak self] in
      await previous?.value
      await job()
      self?.running[key] = nil
    }
    running[key] = task
    tail = task
  }

  private func run(_ batch: WireBatch, epoch: Int, deadline: ContinuousClock.Instant) async {
    guard epoch == self.epoch else { return }
    let began = ContinuousClock.now
    noteFreeze(at: began)
    if resyncDue {
      // The host did not run for a while before this batch. What MusicKit did meanwhile is read
      // as the freeze left it, and reported at the revision it answers, before the batch changes
      // anything.
      resyncDue = false
      resync()
      if current != nil {
        sendStatus()
      }
    }
    revision = batch.revision
    applying = true
    defer { applying = false }
    let endedBefore = ended
    var picked = false
    for action in batch.actions {
      // A newer pick came in while this batch was running.
      guard epoch == self.epoch else { return }
      // MusicKit may have moved on by itself since the last look.
      followMusicKit()
      switch action.action {
      case "set":
        picked = true
        await set(action.ids ?? [], epoch: epoch, deadline: deadline)
      case "add":
        add(action.ids ?? [], epoch: epoch)
      case "remove":
        remove(at: action.index ?? -1)
      case "clear":
        picked = true
        reset()
        #if os(iOS)
          hold.release()
        #endif
      case "skip":
        await skip(
          to: action.index ?? -1, at: max(0, (action.offsetMs ?? 0) / 1000), epoch: epoch,
          deadline: deadline)
      case "start":
        await start(epoch: epoch, deadline: deadline)
      case "stop":
        stop()
      case "setGain":
        // MusicKit's player has no volume of its own; the system volume applies.
        break
      default:
        break
      }
    }
    guard epoch == self.epoch else { return }
    applying = false
    if resyncDue {
      // The host did not run for a while as this batch was applied. What MusicKit did meanwhile
      // counts as it stands, before anything is read into it as live, and what the batch started
      // is its own.
      resyncDue = false
      resync(sparingStartsSince: began)
    } else {
      // Look at the player before answering, so an end is seen once and before the status.
      reconcile()
    }
    // A batch that comes after the queue ended still needs its answer at this revision.
    if endedBefore, ended, !picked {
      sendEnded()
    }
    sendStatus()
  }

  /// Stops MusicKit and forgets every entry.
  private func reset() {
    for entry in entries {
      entry.lookup?.cancel()
    }
    player.stop()
    player.queue = []
    observeQueue()
    entries = []
    stopTimer()
    current = nil
    ended = false
    wantPlaying = false
    startedAt = nil
    sawPlaying = false
    furthest = 0
    stalledSince = nil
    advancing = false
    pendingOffset = nil
    // What MusicKit did with the old queue no longer matters.
    resyncDue = false
  }

  /// The current song goes to MusicKit as soon as it is found; the next one is looked up after
  /// it and joins MusicKit's queue in place once found.
  private func set(_ ids: [String], epoch: Int, deadline: ContinuousClock.Instant) async {
    reset()
    lapseReported = false
    entries = ids.map { Entry(id: $0) }
    guard let first = entries.first else { return }
    startTimer()
    current = 0
    guard MusicAuthorization.currentStatus == .authorized else {
      fail(first, .notAuthorized)
      return
    }
    guard let song = await resolve(first, epoch: epoch, deadline: deadline) else { return }
    replaceQueue(startingWith: first, song)
    for entry in entries.dropFirst() {
      lookUp(entry, epoch: epoch)
    }
    await attempt(on: first, epoch: epoch, deadline: deadline) {
      try await self.player.prepareToPlay()
    }
  }

  /// Appends entries. Nothing starts, and after the end nothing is looked up until a `skip`
  /// makes an entry current.
  private func add(_ ids: [String], epoch: Int) {
    for id in ids {
      let entry = Entry(id: id)
      entries.append(entry)
      startTimer()
      if !ended {
        lookUp(entry, epoch: epoch)
      }
    }
  }

  /// Removes an entry after the current one, from MusicKit's queue too, and stops its lookup so
  /// that it is never inserted. A `remove` aimed at the current entry, as when the host moved on
  /// just as the player changed its next track, is ignored.
  private func remove(at index: Int) {
    guard let current, index > current, entries.indices.contains(index) else { return }
    let entry = entries.remove(at: index)
    entry.lookup?.cancel()
    if entry.queued {
      pruneQueue()
    }
  }

  /// Within the current entry this is a seek. To the next entry it moves MusicKit on in place,
  /// playing or paused as before.
  private func skip(
    to index: Int, at seconds: Double, epoch: Int, deadline: ContinuousClock.Instant
  ) async {
    guard let current, !ended, entries.indices.contains(index) else { return }
    if index == current {
      await seek(to: seconds, epoch: epoch, deadline: deadline)
      return
    }
    guard index == current + 1 else {
      NSLog("%@", "Apple Music host: ignoring a skip from entry \(current) to \(index)")
      return
    }
    let next = entries[index]
    let failedBefore = next.failed
    let song = await song(for: next, epoch: epoch, deadline: deadline)
    guard epoch == self.epoch, self.current == current, self.index(of: next) == index else {
      return
    }
    if let song, !next.failed {
      if stalledSince != nil || advancing {
        // The current entry played out, so a fresh queue costs no gap.
        replaceQueue(startingWith: next, song)
      } else {
        if !next.queued {
          await insert(next, song, epoch: epoch, deadline: deadline)
          guard epoch == self.epoch, self.current == current else { return }
        }
        if next.queued, !next.failed, !musicKitPlays(next) {
          // MusicKit may have moved on by itself as the skip failed.
          await attempt(on: next, epoch: epoch, deadline: deadline, unless: musicKitPlays(next)) {
            try await self.player.skipToNextEntry()
          }
          guard epoch == self.epoch, self.current == current else { return }
        }
      }
    }
    becomeCurrent(index, at: seconds)
    if musicKitPlays(next) {
      pruneBehind()
      forgetBehind()
    }
    guard !next.failed else {
      // The player moved on to an entry this host cannot play: nothing plays, and the player
      // hears why at this revision.
      startedAt = nil
      player.pause()
      if failedBefore, let failure = next.failure {
        sendFailed(index, failure)
      }
      return
    }
    if seconds > 0 {
      player.playbackTime = seconds
      pendingOffset = isPlaying ? nil : seconds
    }
    if !wantPlaying {
      player.pause()
    }
  }

  /// Moves within the current entry. One that played out goes back to MusicKit first, playing
  /// or paused as before.
  private func seek(to seconds: Double, epoch: Int, deadline: ContinuousClock.Instant) async {
    guard let current, let entry = currentEntry, entry.queued, !entry.failed else { return }
    if stalledSince != nil || advancing, let song = entry.song {
      replaceQueue(startingWith: entry, song)
      becomeCurrent(current, at: seconds)
      if wantPlaying {
        await attempt(on: entry, epoch: epoch, deadline: deadline) {
          try await self.player.play()
        }
      } else {
        await attempt(on: entry, epoch: epoch, deadline: deadline) {
          try await self.player.prepareToPlay()
        }
      }
      guard epoch == self.epoch else { return }
    }
    player.playbackTime = seconds
    furthest = seconds
    stalledSince = nil
    // Playback that has not begun yet may drop the seek, so the next start repeats it.
    pendingOffset = isPlaying ? nil : seconds
  }

  private func start(epoch: Int, deadline: ContinuousClock.Instant) async {
    wantPlaying = true
    // An entry that played out is not started again: MusicKit, or the host, moves on from it
    // and plays on.
    guard !ended, stalledSince == nil, !advancing, let entry = currentEntry, entry.queued,
      !entry.failed
    else { return }
    if !isPlaying {
      sawPlaying = false
      startedAt = .now
    }
    await attempt(on: entry, epoch: epoch, deadline: deadline) { try await self.player.play() }
    if let offset = pendingOffset, epoch == self.epoch {
      player.playbackTime = offset
      furthest = offset
      pendingOffset = nil
    }
  }

  private func stop() {
    wantPlaying = false
    stoppedAt = .now
    startedAt = nil
    player.pause()
  }

  /// Makes the entry at `index` current, from `offset`.
  private func becomeCurrent(_ index: Int, at offset: Double = 0) {
    if index != current {
      pendingOffset = nil
    }
    current = index
    sawPlaying = false
    furthest = offset
    stalledSince = nil
    advancing = false
    startedAt = wantPlaying ? .now : nil
  }

  /// Gives MusicKit a fresh queue that starts with `entry`. That pauses what plays, so it is only
  /// done when nothing does. The entries after it follow by inserts, as found songs do.
  private func replaceQueue(startingWith entry: Entry, _ song: Song) {
    player.queue = ApplicationMusicPlayer.Queue(for: [song])
    observeQueue()
    for other in entries {
      other.queued = false
      other.queueEntryID = nil
    }
    entry.queued = true
    entry.queueEntryID = player.queue.currentEntry?.id ?? player.queue.entries.first?.id
    if entry.queueEntryID == nil {
      NSLog("%@", "Apple Music host: a new queue has no entry id yet; its song will tell")
    }
    let epoch = epoch
    enqueue { [weak self] in await self?.insertFound(epoch: epoch) }
  }

  // MARK: - Entries

  /// Looks up an entry's song in the background, and inserts it into MusicKit's queue in line
  /// with the batches once found.
  private func lookUp(_ entry: Entry, epoch: Int) {
    guard entry.song == nil, entry.lookup == nil, !entry.failed else { return }
    entry.lookup = Task { [weak self] in
      guard let self else { return }
      let found = await self.resolve(entry, epoch: epoch, deadline: .now + Self.timeout)
      guard found != nil, epoch == self.epoch, self.index(of: entry) != nil else { return }
      self.enqueue { [weak self] in await self?.insertFound(epoch: epoch) }
    }
  }

  /// The song of an entry, waiting for its lookup until `deadline`. An entry whose song is not
  /// found by then fails.
  private func song(for entry: Entry, epoch: Int, deadline: ContinuousClock.Instant) async
    -> Song?
  {
    if let song = entry.song { return song }
    guard !entry.failed else { return nil }
    guard let lookup = entry.lookup else {
      return await resolve(entry, epoch: epoch, deadline: deadline)
    }
    _ = try? await bounded(until: deadline) { await lookup.value }
    guard epoch == self.epoch, index(of: entry) != nil else { return nil }
    if entry.song == nil, !entry.failed {
      lookup.cancel()
      fail(entry, .network)
    }
    return entry.song
  }

  /// Puts the song found for the first entry after the current one that MusicKit's queue does
  /// not hold yet into it, unless that entry is not found yet or failed, so that MusicKit never
  /// moves on to an entry that cannot be played. The next one goes in line behind the batches
  /// that came meanwhile, so that a pause, a seek or a skip waits for one insert at most.
  private func insertFound(epoch: Int) async {
    guard epoch == self.epoch, !ended, let current, let playing = currentEntry, playing.queued,
      !playing.failed,
      let entry = entries[(current + 1)...].first(where: { !$0.queued }), !entry.failed,
      let song = entry.song
    else { return }
    guard queueSettled else {
      awaitSettling(epoch: epoch)
      return
    }
    await insert(entry, song, epoch: epoch, deadline: .now + Self.insertTimeout)
    guard epoch == self.epoch, entry.queued else { return }
    enqueue { [weak self] in await self?.insertFound(epoch: epoch) }
  }

  /// Whether MusicKit's queue holds no transient entry, that is whether the system's player
  /// service has confirmed every song given to it.
  private var queueSettled: Bool {
    !player.queue.entries.contains(where: \.isTransient)
  }

  /// Waits until MusicKit's queue settles, the pick changes, or `deadline` passes.
  private func queueSettles(epoch: Int, until deadline: ContinuousClock.Instant) async -> Bool {
    while !queueSettled {
      guard epoch == self.epoch, ContinuousClock.now < deadline, !Task.isCancelled else {
        return false
      }
      try? await Task.sleep(for: Self.settlePoll)
    }
    return epoch == self.epoch
  }

  /// The next insert waits for MusicKit's queue to settle outside the line of batches, so that a
  /// pause or a seek still goes through meanwhile, and goes in line once it has. Should the queue
  /// not settle, the next entry is played by moving on by hand.
  private func awaitSettling(epoch: Int) {
    guard settlingEpoch != epoch else { return }
    settlingEpoch = epoch
    Task { [weak self] in
      guard let self else { return }
      let settled = await self.queueSettles(epoch: epoch, until: .now + Self.timeout)
      if self.settlingEpoch == epoch {
        self.settlingEpoch = nil
      }
      guard epoch == self.epoch else { return }
      guard settled else {
        NSLog("%@", "Apple Music host: MusicKit's queue did not settle, so nothing more goes in")
        return
      }
      self.enqueue { [weak self] in await self?.insertFound(epoch: epoch) }
    }
  }

  /// Inserts an entry's song at the tail of MusicKit's queue, which leaves what plays alone, and
  /// remembers which MusicKit entry it became.
  private func insert(
    _ entry: Entry, _ song: Song, epoch: Int, deadline: ContinuousClock.Instant
  ) async {
    // MusicKit puts a song inserted behind an entry it has not confirmed yet at the head of the
    // queue instead, and the system's player service crashes on that.
    let settled = await queueSettles(epoch: epoch, until: deadline)
    guard epoch == self.epoch, index(of: entry) != nil, !entry.queued else { return }
    guard settled else {
      NSLog("%@", "Apple Music host: MusicKit's queue did not settle in time for an insert")
      fail(entry, .network)
      return
    }
    let before = Set(player.queue.entries.map(\.id))
    let work = Task { try await self.player.queue.insert([song], position: .tail) }
    do {
      try await bounded(until: deadline) { try await work.value }
    } catch {
      abandon(work)
      if Self.isCancellation(error) || epoch != self.epoch || index(of: entry) == nil { return }
      NSLog("%@", "Apple Music host: queueing a song failed: \(error)")
      failure(of: error, entry)
      return
    }
    guard epoch == self.epoch, index(of: entry) != nil else {
      pruneQueue()
      return
    }
    entry.queued = true
    entry.queueEntryID = player.queue.entries.last { !before.contains($0.id) }?.id
    if entry.queueEntryID == nil {
      NSLog("%@", "Apple Music host: an inserted song has no new entry id; its song will tell")
    }
  }

  /// An insert that was given up on may still land, and MusicKit would then hold a song that no
  /// entry maps to. What it added is taken out at once, and again once it is done.
  private func abandon(_ work: Task<Void, Error>) {
    work.cancel()
    pruneQueue()
    Task { [weak self] in
      _ = try? await work.value
      self?.enqueue { [weak self] in self?.pruneQueue() }
    }
  }

  /// Takes the entries after the one that plays out of MusicKit's queue when no entry of the host
  /// maps to them: an insert that landed after it was given up on, or an entry that left or
  /// failed. MusicKit never moves on to a song the player did not give or took back.
  private func pruneQueue() {
    let queue = player.queue.entries
    var start = 0
    if let playing = player.queue.currentEntry?.id,
      let position = queue.firstIndex(where: { $0.id == playing })
    {
      start = position + 1
    }
    let mapped = entries.filter { $0.queued && !$0.failed }
    for position in stride(from: queue.count - 1, through: start, by: -1) {
      let queued = queue[position]
      if mapped.contains(where: { plays(queued, $0) }) { continue }
      NSLog("%@", "Apple Music host: taking a queue entry that maps to no track out of the queue")
      player.queue.entries.remove(at: position)
    }
  }

  /// Takes the entries before the one that plays out of MusicKit's queue. They played, or the
  /// player moved on from them, and the system's controls would otherwise go back to them where
  /// the host does not follow; without them, going back starts the current entry again. This
  /// holds on macOS as much as on iOS: Control Center, the media keys and headphones drive
  /// MusicKit there too.
  private func pruneBehind() {
    let queue = player.queue.entries
    guard let playing = player.queue.currentEntry?.id,
      let position = queue.firstIndex(where: { $0.id == playing }), position > queue.startIndex
    else { return }
    player.queue.entries.removeSubrange(queue.startIndex..<position)
  }

  /// The current entry played out while an entry follows it, and MusicKit did not move on: the
  /// next song came too late for its queue, or is still being looked up. The host moves on
  /// itself, or ends when the next entry cannot be played.
  private func advanceByHand(to next: Entry) {
    advancing = true
    let epoch = epoch
    Task { [weak self] in
      guard let self else { return }
      // The lookup is waited for outside the line of batches, so that a pause still goes
      // through meanwhile, and no longer than a batch would. The move itself goes in line.
      _ = await self.song(for: next, epoch: epoch, deadline: .now + Self.batchBudget)
      guard epoch == self.epoch else { return }
      self.enqueue { [weak self] in
        guard let self, epoch == self.epoch else { return }
        defer { self.advancing = false }
        await self.moveOn(to: next, epoch: epoch)
      }
    }
  }

  private func moveOn(to next: Entry, epoch: Int) async {
    followMusicKit()
    // MusicKit moved on by itself, the entry left, or a seek took the current entry back.
    guard !ended, stalledSince != nil, let current, index(of: next) == current + 1 else {
      return
    }
    guard let song = next.song, !next.failed else {
      finish()
      return
    }
    // Nothing plays any more, so a fresh queue costs no gap.
    replaceQueue(startingWith: next, song)
    becomeCurrent(current + 1)
    sendStatus()
    if wantPlaying {
      await attempt(on: next, epoch: epoch, deadline: .now + Self.batchBudget) {
        try await self.player.play()
      }
    }
  }

  // MARK: - Matching MusicKit's entries

  /// The song a MusicKit queue entry plays.
  private static func songID(of queued: MusicPlayer.Queue.Entry) -> MusicItemID? {
    if case .song(let song)? = queued.item {
      return song.id
    }
    return (queued.transientItem as? Song)?.id
  }

  /// Whether MusicKit's `queued` holds `song`. MusicKit may name a song by another id than the
  /// lookup that found it, so the same title and artist count as well.
  private static func holds(_ queued: MusicPlayer.Queue.Entry, _ song: Song) -> Bool {
    if songID(of: queued) == song.id { return true }
    guard queued.title == song.title else { return false }
    if case .song(let item)? = queued.item {
      return item.artistName == song.artistName
    }
    return true
  }

  /// Whether MusicKit's queue holds no entry under the id `entry` knows, so that only its song
  /// can tell which queue entry it is.
  private func lostQueueEntry(_ entry: Entry) -> Bool {
    guard let id = entry.queueEntryID else { return true }
    return !player.queue.entries.contains { $0.id == id }
  }

  /// Whether MusicKit's `queued` plays `entry`: by its id, or else by its song when the id that
  /// `entry` knows is missing or gone, as when MusicKit gave the entry a new one, and no other
  /// entry has that queue entry. A match by the song takes on the new id.
  private func plays(_ queued: MusicPlayer.Queue.Entry, _ entry: Entry) -> Bool {
    guard entry.queued else { return false }
    if queued.id == entry.queueEntryID { return true }
    guard lostQueueEntry(entry), let song = entry.song, Self.holds(queued, song),
      !entries.contains(where: { $0 !== entry && $0.queueEntryID == queued.id })
    else { return false }
    let named = Self.songID(of: queued)?.rawValue ?? "nothing"
    NSLog(
      "%@",
      "Apple Music host: matched a queue entry by its song, as its id was "
        + (entry.queueEntryID == nil ? "unknown" : "gone")
        + "; MusicKit names the song \(named), the lookup \(song.id.rawValue)")
    entry.queueEntryID = queued.id
    return true
  }

  /// Whether MusicKit's current entry plays `entry`.
  private func musicKitPlays(_ entry: Entry) -> Bool {
    guard let queued = player.queue.currentEntry else { return false }
    return plays(queued, entry)
  }

  // MARK: - MusicKit calls

  /// Runs one player call for `entry`. A failure is reported unless it only means that a newer
  /// pick took over, or `done` says that what the call was for happened anyway.
  private func attempt(
    on entry: Entry, epoch: Int, deadline: ContinuousClock.Instant,
    unless done: @autoclosure () -> Bool = false,
    _ work: @escaping @MainActor @Sendable () async throws -> Void
  ) async {
    do {
      try await bounded(until: deadline, work)
    } catch {
      if Self.isCancellation(error) || epoch != self.epoch || done() { return }
      NSLog("%@", "Apple Music host: playback failed: \(error)")
      failure(of: error, entry)
    }
  }

  /// A queue that replaces one that is still starting interrupts its `play()`; that is routine.
  private static func isCancellation(_ error: Error) -> Bool {
    if error is CancellationError { return true }
    let nsError = error as NSError
    return nsError.domain == "MPMusicPlayerControllerErrorDomain" && nsError.code == 2
  }

  /// Reports a failed MusicKit call. Unplayable tells the player that the track is gone from
  /// the catalogue, so only what says so maps to it. `play()` surfaces MediaPlayer errors, so
  /// MPErrorDomain is read as well as the URL and MusicKit errors.
  private func failure(of error: Error, _ entry: Entry) {
    if error is TimedOut, interrupted(entry) { return }
    if error is TimedOut || error is PlaybackStopped {
      fail(entry, .network)
      return
    }
    if MusicAuthorization.currentStatus != .authorized || error is MusicTokenRequestError {
      fail(entry, .notAuthorized)
      return
    }
    if let request = error as? MusicDataRequest.Error {
      switch request.status {
      case 401, 403: fail(entry, .notAuthorized)
      case 429, 500...599: fail(entry, .network)
      default: fail(entry, .unplayable)
      }
      return
    }
    let nsError = error as NSError
    switch (nsError.domain, nsError.code) {
    case (NSURLErrorDomain, _), ("MPErrorDomain", 3), ("MPErrorDomain", 7):
      fail(entry, .network)
    case ("MPErrorDomain", 1):
      fail(entry, .notAuthorized)
    case ("MPErrorDomain", 2):
      // The subscription lapsed, so nothing can be played any more. The queue goes at once, so
      // the batch stops here, and the host starts again as one that cannot play, once.
      guard !lapseReported else { return }
      lapseReported = true
      drop()
      lapsed()
    default:
      if let sub = error as? MusicSubscription.Error, sub == .permissionDenied {
        fail(entry, .notAuthorized)
      } else if let library = error as? MusicLibrary.Error, library == .permissionDenied {
        fail(entry, .notAuthorized)
      } else if interrupted(entry) {
        return
      } else {
        fail(entry, .unplayable)
      }
    }
  }

  /// Whether a failure only means that the system interrupted the audio, as a call or another
  /// app does. The entry is then not failed, and the start limit waits for the interruption to
  /// end. MusicKit documents no error for it, so the player's status is what tells.
  private func interrupted(_ entry: Entry) -> Bool {
    guard player.state.playbackStatus == .interrupted else { return false }
    NSLog("%@", "Apple Music host: a call failed while the audio is interrupted")
    startedAt = entry === currentEntry && wantPlaying ? .now : startedAt
    return true
  }

  // MARK: - Songs

  /// The song of an entry. One that cannot be found, or whose lookup fails, fails the entry,
  /// unless the entry left the queue meanwhile.
  private func resolve(_ entry: Entry, epoch: Int, deadline: ContinuousClock.Instant) async
    -> Song?
  {
    if let song = entry.song { return song }
    let id = entry.id
    if let cached = songs[id] {
      entry.song = cached
      return cached
    }
    do {
      let found = try await bounded(until: deadline) { try await self.lookups.song(id) }
      guard epoch == self.epoch, index(of: entry) != nil else { return nil }
      guard let found else {
        let source =
          id.hasPrefix("catalog:")
          ? "the catalogue does not have"
          : "neither this device's library nor the catalogue has"
        NSLog("%@", "Apple Music host: \(source) song \(id)")
        fail(entry, .unplayable)
        return nil
      }
      if songs.count > 2000 { songs.removeAll() }
      songs[id] = found
      entry.song = found
      return found
    } catch {
      if Self.isCancellation(error) || epoch != self.epoch || index(of: entry) == nil {
        return nil
      }
      NSLog("%@", "Apple Music host: looking up a song failed: \(error)")
      failure(of: error, entry)
      return nil
    }
  }

  // MARK: - Events

  /// Reports that an entry cannot be played, once. MusicKit never moves on to it.
  private func fail(_ entry: Entry, _ reason: Failure) {
    guard !entry.failed, let index = index(of: entry) else { return }
    entry.failure = reason
    entry.lookup?.cancel()
    sendFailed(index, reason)
    if entry.queued, index != current {
      pruneQueue()
    }
  }

  private func sendFailed(_ index: Int, _ reason: Failure) {
    send(
      encode([
        "type": "jukeboxItemFailed",
        "host": "appleMusic",
        "revision": revision,
        "index": index,
        "reason": reason.rawValue,
      ]))
  }

  /// Reports the end of the queue at the entry that ended, so that a player that did not hear
  /// the host move on meanwhile counts the entries before it as played.
  private func sendEnded() {
    var event: [String: Any] = ["type": "jukeboxEnded", "host": "appleMusic", "revision": revision]
    if let current {
      event["index"] = current
    }
    send(encode(event))
  }

  /// Whether the current entry plays, as Rune should see it. One that starts, or that moves on
  /// to the next entry, still plays.
  private var reportsPlaying: Bool {
    guard !ended, let entry = currentEntry, entry.queued, !entry.failed, wantPlaying else {
      return false
    }
    return isPlaying || !sawPlaying || stalledSince != nil || advancing
  }

  private var position: Double {
    guard let entry = currentEntry, entry.queued, !entry.failed else { return 0 }
    let time = max(0, player.playbackTime)
    return ended || stalledSince != nil ? max(time, furthest) : time
  }

  private func sendStatus() {
    reportedSignature = signature
    reportedAt = .now
    send(
      encode([
        "type": "jukeboxStatus",
        "host": "appleMusic",
        "revision": revision,
        "currentIndex": current.map { $0 as Any } ?? NSNull(),
        "currentId": currentEntry.map { $0.id as Any } ?? NSNull(),
        "playing": reportsPlaying,
        "positionMs": Int((position * 1000).rounded()),
      ]))
  }

  /// What Rune can see of the player, so a change it did not cause is noticed.
  private var signature: String {
    "\(String(describing: current))|\(ended)|\(reportsPlaying)|\(currentEntry?.failed ?? false)"
  }

  /// Notices the host moving on, ending or failing, and reports a change that Rune did not
  /// cause. Nothing is sent while a batch is applied: the batch answers once it is done.
  private func check(periodic: Bool) {
    let now = ContinuousClock.now
    noteFreeze(at: now)
    if periodic {
      lastTick = now
      #if os(iOS)
        releaseHoldIfSettled()
      #endif
    }
    guard !applying else { return }
    if resyncDue {
      resyncDue = false
      resync()
      if current != nil {
        sendStatus()
      }
      return
    }
    reconcile()
    guard current != nil else { return }
    let due = periodic && reportsPlaying && ContinuousClock.now - reportedAt >= Self.reportEvery
    if signature != reportedSignature || due {
      sendStatus()
    }
  }

  /// Follows MusicKit when its current entry changed to a later entry's: the host moved on by
  /// itself, at the end of an entry, through the system's media controls, or by several entries
  /// while it could not run.
  private func followMusicKit() {
    guard !ended, let current, let entry = currentEntry, entry.queued,
      let queued = player.queue.currentEntry
    else { return }
    if queued.id == entry.queueEntryID { return }
    let later = entries.indices.dropFirst(current + 1).filter { entries[$0].queued }
    if let index = later.first(where: { entries[$0].queueEntryID == queued.id }) {
      moved(to: index)
      return
    }
    // Without a match by id the song tells, the current entry's first, so that the same song
    // twice is never taken for a move.
    if plays(queued, entry) { return }
    if let index = later.first(where: { plays(queued, entries[$0]) }) {
      moved(to: index)
      return
    }
    if lostQueueEntry(entry) {
      // Nothing MusicKit was given plays before the current entry.
      NSLog("%@", "Apple Music host: matched the playing queue entry by its place, not id or song")
      entry.queueEntryID = queued.id
      return
    }
    if unmatchedEntryID != queued.id {
      unmatchedEntryID = queued.id
      NSLog("%@", "Apple Music host: MusicKit plays neither the current nor the next track's entry")
    }
  }

  /// MusicKit moved on to the entry at `index` by itself. The entries it played go out of its
  /// queue.
  private func moved(to index: Int) {
    becomeCurrent(index)
    pruneBehind()
    forgetBehind()
  }

  /// Forgets where the entries before the current one stood in MusicKit's queue, which no
  /// longer holds them. The entries keep their indexes, so they stay, but nothing in MusicKit's
  /// queue is matched to them again, however long the run gets.
  private func forgetBehind() {
    guard let current else { return }
    for entry in entries.prefix(current) where entry.queued || entry.queueEntryID != nil {
      entry.queued = false
      entry.queueEntryID = nil
    }
  }

  private func reconcile() {
    followMusicKit()
    guard !applying, !advancing, !ended, let entry = currentEntry, !entry.failed, entry.queued
    else { return }
    let now = ContinuousClock.now
    let time = max(0, player.playbackTime)
    if isPlaying {
      // A resume from outside Rune, such as the system's media controls.
      if !wantPlaying, now - stoppedAt > .seconds(1) {
        wantPlaying = true
      }
      // It plays from where it stands, so no start repeats an earlier seek.
      pendingOffset = nil
      sawPlaying = true
      startedAt = nil
      furthest = max(furthest, time)
      stalledSince = nil
      return
    }
    if player.state.playbackStatus == .interrupted {
      // Another app or a call holds the audio. The start limit counts from the end of that.
      if startedAt != nil { startedAt = now }
    } else if wantPlaying, !sawPlaying, let startedAt, now - startedAt >= Self.startLimit {
      // Told to play, but never heard playing.
      self.startedAt = nil
      NSLog("%@", "Apple Music host: playback did not start in time")
      fail(entry, .network)
      return
    }
    // MusicKit plays something else, or nothing was heard yet.
    let playing = player.queue.currentEntry?.id
    guard sawPlaying, playing == nil || playing == entry.queueEntryID else { return }
    let status = player.state.playbackStatus
    let reached = max(time, furthest)
    let playedOut: Bool
    if let duration = entry.song?.duration, duration > 0 {
      // Playing out ends the entry whether MusicKit then shows stopped or paused.
      playedOut = reached >= duration - Self.endTolerance
    } else {
      playedOut = status == .stopped
    }
    guard playedOut else {
      if status == .stopped {
        // Stopped well before the end, with no stop or clear from Rune.
        failure(of: PlaybackStopped(), entry)
      }
      return
    }
    guard let next = nextEntry, !next.failed else {
      finish()
      return
    }
    // MusicKit moves on to an entry in its queue by itself; give it a moment before helping.
    guard let since = stalledSince else {
      stalledSince = now
      return
    }
    if now - since >= .seconds(1) {
      advanceByHand(to: next)
    }
  }

  /// The last entry finished, and nothing playable follows it.
  private func finish() {
    ended = true
    stalledSince = nil
    sendEnded()
    sendStatus()
  }

  // MARK: - Catching up after a suspension

  /// Notes a time that the host did not run, as in an app that the system suspended: the
  /// periodic check, which runs every half second while the host holds entries, last ran long
  /// ago. Whatever runs first once the host runs again notices it, before it reads anything
  /// into what MusicKit did meanwhile.
  private func noteFreeze(at now: ContinuousClock.Instant) {
    guard let lastTick, now - lastTick > Freeze.gap else { return }
    resyncDue = true
    resumedAt = now
    self.lastTick = now
  }

  /// Takes what MusicKit did while the host did not run as it stands, for the caller to report
  /// at once. MusicKit may have moved on by several entries, played its queue out, or been paused
  /// or stopped from the system's controls. A stop that happened meanwhile is not a failure:
  /// at the last entry that MusicKit holds it is the end, when the entry reached its end or Rune
  /// wanted it to play, and anywhere else it leaves the entry paused where it stands. An entry
  /// that was started once the host ran again, or by the batch that was being applied, from
  /// `since` on, waits for MusicKit as any start does, with its full time.
  private func resync(sparingStartsSince since: ContinuousClock.Instant? = nil) {
    NSLog("%@", "Apple Music host: the host did not run for a while, and follows MusicKit")
    // Told apart before MusicKit's own moves are followed, as those start the entries they make
    // current anew. An entry that MusicKit moved on to by itself is judged whatever started
    // before it.
    let spareFrom = [resumedAt, since].compactMap { $0 }.min()
    let spared = startedAt.map { started in spareFrom.map { started >= $0 } ?? false } ?? false
    let startedEntry = current
    followMusicKit()
    guard !ended, let entry = currentEntry, entry.queued, !entry.failed else { return }
    if spared, current == startedEntry {
      if let startedAt, let resumedAt, startedAt < resumedAt {
        self.startedAt = .now
      }
      return
    }
    stalledSince = nil
    // What was told to play played, or stopped meanwhile: nothing waits for it to start.
    startedAt = nil
    if isPlaying {
      sawPlaying = true
      return
    }
    guard player.state.playbackStatus != .interrupted else { return }
    guard playedOut(entry) else {
      // Rune hears it paused there, and a play starts it again.
      wantPlaying = false
      stoppedAt = .now
      sawPlaying = false
      return
    }
    if let last = entries.lastIndex(where: { $0.queued && !$0.failed }), last != current {
      becomeCurrent(last)
    }
    if let next = nextEntry, !next.failed {
      // The next entry did not reach MusicKit in time, so the host moves on itself.
      sawPlaying = true
      stalledSince = .now
      advanceByHand(to: next)
    } else {
      ended = true
      sendEnded()
    }
  }

  /// Whether `entry`, the current one, played out while the host did not run: MusicKit has
  /// nothing current any more, or it stands at the last entry it holds and reached its end, or,
  /// as Rune wants the entry to play, stopped there or went back to its start, as MusicKit does
  /// once its queue runs out. An entry that Rune paused stays paused, wherever it stands.
  private func playedOut(_ entry: Entry) -> Bool {
    if player.queue.currentEntry == nil { return true }
    guard current == entries.lastIndex(where: { $0.queued && !$0.failed }) else { return false }
    let time = max(0, player.playbackTime)
    if let duration = entry.song?.duration, duration > 0, time >= duration - Self.endTolerance {
      return true
    }
    return wantPlaying
      && (player.state.playbackStatus == .stopped || time < Self.startTolerance)
  }

  // MARK: - Background hold (iOS)

  #if os(iOS)
    /// Holds the app awake as it stops being active while a track after the current one has yet
    /// to reach MusicKit. Once the system suspends the app, MusicKit plays what it holds and
    /// stops.
    private func observeActivity() {
      let names = [
        UIApplication.willResignActiveNotification, UIApplication.didEnterBackgroundNotification,
      ]
      for name in names {
        let observer = NotificationCenter.default.addObserver(
          forName: name, object: nil, queue: .main
        ) { [weak self] _ in
          MainActor.assumeIsolated {
            guard let self, self.pendingAhead else { return }
            self.hold.take()
          }
        }
        activityObservers.append(observer)
      }
    }

    /// Whether an entry after the current one has yet to reach MusicKit: it is neither in its
    /// queue nor failed. After the end nothing more goes to MusicKit, as the entries added then
    /// are not looked up, so none is waited for.
    private var pendingAhead: Bool {
      guard let current, !ended else { return false }
      return entries.dropFirst(current + 1).contains { !$0.queued && !$0.failed }
    }

    /// Lets the app go once no batch is being applied and every entry after the current one
    /// reached MusicKit or failed.
    private func releaseHoldIfSettled() {
      hold.releaseIfDone(settled: !applying && !pendingAhead)
    }
  #endif
}

extension String {
  nonisolated fileprivate func removingPrefix(_ prefix: String) -> String? {
    hasPrefix(prefix) ? String(dropFirst(prefix.count)) : nil
  }
}

/// Whether the device has a network path, as the system reports it. Read from any thread.
nonisolated final class NetworkReach: @unchecked Sendable {
  static let shared = NetworkReach()

  private let monitor = NWPathMonitor()
  private let lock = NSLock()
  private var online = true
  private var started = false

  /// False while the system has no usable path. Until it says, the device counts as online.
  var isOnline: Bool {
    lock.lock()
    defer { lock.unlock() }
    return online
  }

  /// Starts watching, once. `changed` runs whenever the path becomes or stops being usable.
  func start(changed: @escaping @Sendable () -> Void) {
    lock.lock()
    guard !started else {
      lock.unlock()
      return
    }
    started = true
    lock.unlock()
    monitor.pathUpdateHandler = { [weak self] path in
      guard let self else { return }
      let now = path.status == .satisfied
      self.lock.lock()
      let before = self.online
      self.online = now
      self.lock.unlock()
      if now != before { changed() }
    }
    monitor.start(queue: DispatchQueue(label: "ci.not.rune.apple_music.network"))
  }
}
