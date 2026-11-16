# Rune interconnect architecture

This document describes how Rune devices share libraries, play audio and control each other. Library replication is specified in [Rune synchronization integration](sync-integration.md), and the visualizer feed for Subsonic-compatible servers in [runeVisualizerFeed OpenSubsonic extension](opensubsonic-rune-visualizer-feed.md).

## 1. Libraries, players and controllers

### The UI talks to a library and a player

Searching and browsing ask the library. Play, pause, seek and the queue go to the player. Which one a call goes to follows from what the call is.

Each is one trait with several implementations, written as an enum:

- **Library** (the data owner):
    - this device's library;
    - another Rune device's library, kept here as a replica;
    - a Subsonic library.
- **Player**:
    - this device's player;
    - another Rune device's player, reached over the Rune protocol;
    - a Subsonic jukebox.

Calls that are neither, such as pairing and this device's settings, always run on this device.

**The UI shows one player, the current one.** When another player becomes current (Play on, or opening a Subsonic library in remote-control mode or an @RR library), this device's own session pauses where it is and stays out of the UI. When this device's player is current again, the UI shows that session, paused where it stopped.

**Forwarding happens inside the implementations.** There is no routing layer above them:

- Another Rune device forwards every call by default and lists the calls it does not support.
- Subsonic lists the calls it supports and how each one translates to the Subsonic API. Any other call fails with a typed "unsupported" error before any HTTP request.

### The node

Each app process has one node, created at start. It owns the library pool, trust, discovery, grants, the server, the sync worker and the player session. Sharing and sync therefore keep running on the welcome screen, across library switches and while this device controls another one.

### Requests

- Every call is listed once in the request catalogue: its request and response, whether it is a library, player or this-device call, the access rule for calls from other devices, and the protocol version that introduced it.
- The requests are an API for any client, not only the Dart UI. Every request and reply can carry a call id.
- Every failed request answers with a typed failure. Only an internal error raises a crash report. The UI shows an expected failure in place, in the user's language; a global handler shows any failure its caller does not handle.
- Paths in replies are built for the requester: a request from another device gets URLs it can fetch from this device, never this device's file paths.

### Rules for messages

- A choice between known values is an enum, on the wire and in storage. It is never a string, a number, a sentinel value, a set of booleans, or several optional fields of which one is set.
- Nothing is matched by name. Messages, requests and items are matched by type or by id.
- Every piece of state has one writer. A component that has several inputs for one state combines them itself and publishes one result.
- An expected failure is typed from where it happens to where the user sees it.

### Player side and controller side

The device that plays is the player side. Any UI that issues commands to it is a controller, whether it runs on the same device or on another one. The two differ only in the protocol between them. Several controllers can share one player, and all of them are equal:

- The player is the single writer of its state. It applies commands in arrival order, so play, pause, seek and volume follow the last command received.
- A player request answers only after the player has applied the command.
- Queue edits name the queue revision they were made against. An edit against a stale revision is rejected; the controller refreshes and retries. This keeps "remove item 5" from removing the wrong song after someone else inserted one.
- Status is pushed. Every controller receives the same status and queue updates, including changes made by others. A controller on another device receives only the messages a controller renders.
- The player's owner can detach any controller. Revoking Control detaches it permanently.

A node has exactly one player session. There is no API to create more. Its queue, position and playback mode survive a restart.

### Items

The player reads its own media. A controller never decides how a player reads a track; it only names what to play. Items are named as `(library_id, track)`, so a queue can mix libraries and survives a library switch.

Rows of the library open in the UI (track lists, search, collections, the mix editor) may name tracks by that library's integer ids. Everything that can show a track of another library (the queue, now playing, likes, lyrics, loads, cast, handoff) uses `(library_id, track)`.

Playing a mix asks the library to resolve the mix query and then loads the result into the player.

### Cast

Casting makes another device's player the current player. The library stays on this device, so searching while casting still asks this device's library, and the play bar shows the other device's progress, pushed back over the Rune connection.

- A cast is a load command, not an audio stream. The sender sends the target player a list of resolved items. The target must already hold a standing read grant on each item's library (chapter 5); a cast creates no access of its own. The target resolves the items and fetches the bytes itself.
- A cast sends items, never a query. Casting a mix resolves the mix query on this device and sends the result, because the query depends on this device's catalog. Large results are sent in batches.
- Playback depends on the source staying reachable. If the device that holds the library leaves the network, the target can continue only with items it has buffered or kept as offline copies.
- Before sending, this device runs a preflight check so the user sees whether the cast can work (chapter 5).
- If the target may read a library but its copy does not hold some of the cast tracks yet, it loads the tracks it holds and starts playing; the user is told how many tracks were left out because the target does not have them yet.

While another device is the player, its status and queue replace this device's in the UI, and this device's own session is paused. If the connection drops, the UI shows this device's own session again and says that the connection was lost.

### Handoff

Handoff moves the queue to another player.

- **The target keeps its own queue.** The handed-off queue is inserted after the target's current track, and playback continues from the handed-off current track at the handed-off position. Example: the target holds A B C D E and plays C; handing off 1 2 3 4 5 while playing 3 gives A B C 1 2 3 4 5 D E, playing 3 from its position. A target with no current track appends the handed-off queue at the end; an empty target simply takes it.
- **This device pauses first.** Pressing handoff pauses this device at once and records the position; the target starts from that position. There may be a short silence while the target loads, but nothing is heard twice. If the target fails to load, this device resumes from where it paused and nothing changes on the target.
- **A handoff completes when the target plays.** If the target accepts the queue but then cannot play the handed-off track, the handoff is rolled back the same way: the inserted block is taken back and this device resumes. The source waits for the target's status for the handed-off track, bounded by the request timeout.
- **A handoff overtaken by another fails** with a typed conflict.
- **Handing back takes back only what was handed in.** The inserted block is tagged with the handoff it came from. Handing back takes that block only, including edits made to it while controlling, and leaves the target with its own queue (A B C D E):
    - If the target is playing inside the block, this device continues from that track and position, and the target is paused where its own track (C) was interrupted.
    - If the target has already played past the block and plays its own tracks (D), the block comes back stopped at its end, and the target keeps playing D.
- The device that gives up playback keeps its paused queue; a block taken back replaces it.
- A Subsonic jukebox plays only its server's library, so nothing is handed off to it.

### Three ways to use another Rune device

- **Replica**: sync its library to this device, then browse and play here.
- **Play on**: browse here, play there (cast and handoff).
- **@RR exclusive mode**: this app becomes the other device's UI, for example for a headless device. Library and player calls go to that device; this device's settings and this-device calls stay here.

## 2. Capabilities, addresses and the visualizer

### Capabilities

Each implementation reports what it can do, and the UI is gated on that rather than on a mode flag.

- A library reports `browse`, `stream`, `download` (the client may keep bytes), `sync`, and the Rune extensions it offers (the visualizer feed, charts, mixes, recommendations). A Rune device offers all extensions; other servers declare each one.
- A player reports:
    - `accepted_sources`: any library it holds a read grant for, or its own library only. Casting is valid only to a player that accepts any granted library.
    - `sessions`: one shared session or several.
    - `status_delivery`: push, or poll with an interval.
    - `controls`: the supported commands, such as seek, volume or gain, and queue edits.
- The capability snapshot combines these with the implementation's supported calls and, for a controller on another device, with its grants. Dart hides unsupported controls and disables unavailable ones with a reason. Rust checks again on every call.

### Discovery and protocol version

- Devices announce their name, version, model, type, certificate fingerprint, port and protocol version over UDP multicast. The device type is descriptive only.
- A public device-info endpoint publishes the same metadata and the capabilities, and nothing about libraries.
- The Rune protocol has its own semantic version, checked in the WebSocket handshake. A major mismatch is a typed "needs update" error naming which side is older; a minor difference narrows the capability snapshot; a pre-release is compatible only with the identical version.

### Subsonic

- Adding a Subsonic server adds a library. After login the account's roles decide which modes are offered, and the user picks one:
    - **Play on this device** (needs streamRole): browse and search ask the server, and audio streams to this device's player, so everything built on local playback works.
    - **Remote control** (needs jukeboxRole): browse and search ask the server, and playback controls drive the server's jukebox. The visualizer works only when the server declares `runeVisualizerFeed`.
    - The mode is fixed when the library is added. To use the other mode, the server is added again as a second library.
- As a library: browse, search and stream need streamRole; download needs downloadRole; `sync` is always false. Mixes are not offered for Subsonic libraries, so the directory tree (used only by the mix editor) is not either. Scan, analysis, deduplication and primary colour stay unsupported because the server owns them. Covers come from `getCoverArt` and are cached on this device.
- As a player: the jukebox accepts only its own library, has one shared session, is polled for status, seeks through `skip` with an offset and uses `setGain` for volume. Without a server-declared guarantee, queue edits are best effort. The jukebox's player implementation is the only writer of its status; the visualizer feed, when the server offers it, is one of its inputs.
- Grants on a Subsonic server belong to the user account. The account's roles become capability bits; Rune's own access control (chapter 5) stays per device.
- When this device plays a Subsonic track, it reports now-playing and the completed play to the server.
- When the server rejects the stored credentials, the user is asked to log in again.

### Addresses

- Every endpoint has one URL form, parsed by one parser. Inside a node, libraries and devices are typed records; the URL is only the form in which an address is entered or exchanged.
- The endpoint URL grammar: the scheme names the implementation and `+` the transport.
    - `rune://<fingerprint>?v=1&addr=192.168.1.5:7863` for Rune devices: identity first, addresses as hints. The fingerprint is the SHA-256 of the public key in lowercase base32.
    - `subsonic+https://music.example.com/navidrome?v=1&user=alice` for Subsonic servers.
    - `opendal+s3://bucket/prefix?v=1&region=…` and `opendal+webdav://…` for OpenDAL storage.
    - `file://` and Android `content://` for local libraries.
- `v` is required, and a newer `v` reports that a newer Rune is needed. Every parameter must be understood: an unknown parameter is an error that names it, except parameters starting with `opt-`, which a reader may ignore. Parse errors are typed.
- A device address is one type: a host or IP and a port, with one parser, one printer, one default port and one way to build a device's URLs. A discovered device is added from the fingerprint and addresses it announces, a manually added one from a host and port typed into a form; both are confirmed with the fingerprint quiz. Pairing trusts a device only when the certificate the user verified is the one the device presented. Connections between devices always use TLS.

### Secrets

Secrets never appear in a URL. Passwords, API keys and storage credentials live in one credentials file in the node's config directory, encrypted with a random key kept in a separate 0600 file next to it; there are no platform keychains, and the same code runs on every platform. Library records hold only a credential reference. Secrets never enter a catalog DB, a portable `.rune/` folder, a share or anything synced, and logs, URLs and errors never contain one. The one exception is the Subsonic protocol itself, which carries its authentication in the query of every call.

### Visualizer

The visualizer always renders on the device that shows it, so a controller can draw the visualizer for a player on another device when it receives the inputs:

- the song clock: the current item, position and playing state. The receiver advances the position from the time it received each status;
- the track chart: beats, bars, meter, sections, onset streams and loudness curves found by analysing the whole file, produced once per track;
- the realtime spectrum, produced while decoding;
- covers: the playing item's cover and a set of other covers for scenes that show a cover wall.

When this device plays, including streaming from Subsonic, the clock and spectrum come from the local player and the chart is analysed locally. When another device plays, all four inputs come from that player through the feed. The feed is a read-only subset of the Rune protocol: subscribe and unsubscribe, enabling the spectrum, and the status, chart, cover and spectrum messages. The player pushes the chart once per track as soon as its analysis finishes, so a controller never needs the audio file. A Rune player offers the feed to any attached controller; any other server must declare it.

The covers come from the playing item's own library implementation; a Subsonic library supplies them from its cover cache.

Scenes declare the inputs they need: the spectrum, or the spectrum and the chart. Which scenes are offered depends on what the current source provides, decided when the source changes:

- this device plays, or a Rune player or a Subsonic server with `runeVisualizerFeed` sends the feed: spectrum and chart, the full visualizer;
- a source that declares it sends the spectrum but no charts: only scenes that need nothing but the spectrum;
- no feed: no visualizer.

A chart that is still being analysed for the current track does not change the offered scenes: the scene draws with the spectrum until the chart arrives. Loading a scene the source cannot feed fails with a typed error.

### Rune extensions on Subsonic servers

A Subsonic-compatible server declares Rune extensions through the OpenSubsonic extension mechanism:

1. Every OpenSubsonic response carries `openSubsonic: true`, `type` and `serverVersion`. A classic Subsonic server without these fields never gets Rune features.
2. `GET /rest/getOpenSubsonicExtensions` returns `[{ name, versions }]`.
3. A server that supports the feed lists `{ "name": "runeVisualizerFeed", "versions": [1] }`. Version 1 adds one endpoint that returns the feed's WebSocket URL; the handshake authenticates with the same Subsonic credentials as every other call. The extension is specified in [runeVisualizerFeed OpenSubsonic extension](opensubsonic-rune-visualizer-feed.md).
4. The client reads the list again whenever `serverVersion` changes.

Extensions are declared one by one, so a server can implement only some of them. Names are camelCase with a `rune` prefix, like existing OpenSubsonic names.

## 3. Storage and the share

### Storage backends

Storage describes where bytes live and how they are reached. Each library binds to one storage backend and a root:

- local paths on Windows, Linux and macOS;
- Android Storage Access Framework tree URIs;
- WebDAV and S3 through Apache OpenDAL; other OpenDAL services can follow.

The backend is chosen per library, so one node can hold a local, a SAF and an OpenDAL library at the same time. Audio analysis of an OpenDAL library is the same background task as for a local one, reading through the library's backend; it never blocks playback.

A scan decides whether a file changed by the backend's change token (an ETag on WebDAV and S3), kept apart from the file's modification time. A file without a change token is described again on every scan.

A replica does not use the owner's storage: it fetches bytes from the owning device over that device's authenticated routes.

An OpenDAL backend has two duties for a library, decided separately:

- **Media source.** Rune reads and lists media from it. The UI offers no write operations on it, such as tag editing, deletion or cover embedding.
- **Keeping the library's data in the share** (writable backends only, the user chooses). The live catalog DB always stays on the device, because SQLite cannot run on these services. The share holds the library's data as a sync source in the neutral sync format: every device that opens the share pushes its changes there and pulls other devices' changes from it. Rune never opens a database inside a share. A read-only share holds no sync data.

### Share sync

- Every object has exactly one writer, except `library.json`, which is created once. Each device writes only its own folder and reads every device's folder; each device merges into its own local SQLite, and nothing is merged inside the share.
- The share holds one current state (a base) plus at most about 30 days of per-device change segments. It never holds a growing history.
- The sync `Coordinator`, `ReceiveStore`, `SnapshotSink` and `SyncPeer` traits are unchanged; share sync uses only the pull direction.

Layout:

```
<root>/.rune/sync/library.json                    {format, library_id, model, schema}; created once
<root>/.rune/sync/devices/<stream>.<generation>/   written only by its owner
    head.0.json, head.1.json                      alternating head slots, self-checksummed
    seg/<first>-<last>-<hash>.gz                  immutable change segments, levels L0 to L2
    base/<base_id>.json                           immutable base manifest
    obj/<blake3>.gz                               immutable base chunks and context fragments
```

- `stream = uuid_v5(local replica, "rune-share")`, distinct from the replica id, so share checkpoints never collide with LAN checkpoints. `generation` is a fresh UUID at creation and after a clone or restore; each `<stream>.<generation>` prefix is an independent store identity.
- A head (about 4 KB) holds the identity, the writer session and counter, `head` and `floor` checkpoints, the floor base, up to about 35 segment references, the device's bases, acknowledgements of other streams, its local cursor, retired prefixes and a trailing checksum.
- A segment is gzip JSON of a run of chained events; the reader recomputes every event digest and anchor.
- A base manifest records which checkpoint of every stream it covers and, per collection, a root summary and 2^k bucket summaries with their chunk names. `base_id` is the blake3 of the manifest. Chunks hold one bucket's canonical candidates, about 1 MiB gzip, named by the blake3 of their bytes; k changes only when the average chunk leaves 0.5 to 2 MiB, so unchanged buckets keep their names.

### Push

Push publishes this device's stream.

- Triggers: 10 s after the last write when a non-statistics change is pending; at most every 30 min when only statistics are pending; when the app goes to the background or exits. Pull always runs first.
- Only local writes and changes that came from neither a share stream nor a device's own local replica are published, so data relayed from LAN peers reaches the share once.
- The segment is uploaded first, then the head through the store's conditional write; the head write is the commit. The export cursor advances only after a confirmed head write.
- 16 L0 segments roll up into one L1, and 16 L1 into one L2; replaced segments are deleted 24 h later.
- A large change (above 16 MiB gzip or 25% of the newest base) cuts a base instead of uploading segments.

### Base cut

- The keeper is the smallest stream id among streams whose head changed in the last 7 days. It cuts when the newest base is 30 days old or the retained tail exceeds 25% of the base. Any device cuts when the newest base is more than 45 days old.
- The keeper pulls and pushes first, exports from its local store so summaries match receivers byte for byte, uploads only chunks absent from its retained manifests, then the manifest, then the head.
- A base is superseded when another live base covers every stream at least as far; its owner deletes it 7 days later.

### Pull

- List `devices/` every 10 minutes, or when a head names an unknown prefix. Ignore heads of another library. Poll heads with a conditional GET: every 60 s in the foreground, every 15 min in the desktop background.
- For each stream, the sync coordinator reads the segments after this device's checkpoint. A stream below its floor, or met for the first time, reconciles against a base: first contact starts with the base whose coverage is largest, and a stream whose floor base is gone reconciles against the best live base in the current heads. Only chunks whose summaries differ are downloaded.
- A stream with its whole chain present (floor at 0) is adopted without reconciliation. After reconciling against a base, the checkpoints it covers are adopted (raise only).

### Garbage collection and retirement

Own-prefix garbage collection runs through the same writer as push and deletes only this device's objects: segments every active stream has acknowledged or older than 30 days, superseded bases, and objects two runs at least 24 h apart both saw unreferenced. A stream whose head has not changed for 180 days is retired by the keeper, which deletes that prefix once one of its live bases covers the retired head.

### Ordering, time and integrity

- Within a stream, order comes from the sequence number and the blake3 anchor chain. Across streams no order is needed: the merge is order-independent. Statistics keys are per track and device, so statistics never conflict.
- Wall time is used only for debounce, waiting periods and display. Every period is elapsed time on one device's own clock, persisted locally and restarted when a stored start lies in the future. An S3 clock-skew rejection is a typed "device clock wrong" state.
- Objects are written before the head that names them, and every object is verified by digest, chain or checksum. The two head slots survive a torn WebDAV write.
- A clone or restore is detected (foreign writer session, counter mismatch, an S3 412 that is not this device's own write, or a head ahead of the local journal) and rotates to a new generation.
- Every format carries a version; a device that sees a newer format stops all writes and keeps reading.
- One writer per prefix also inside a device: push, acknowledgement-only head writes, roll-up, base cut and garbage collection go through one serialized writer.
- Only a real 404 or a digest mismatch can trigger reconciliation, `library.json` creation, rotation or retirement. 423, 5xx, timeouts and failed listings are transient typed errors with backoff.
- `library.json` is re-validated on every pass. If it disappears or names another library under an existing binding, the device stops with a typed state and never recreates it.
- On a store without create-if-absent, `library.json` is written with a plain PUT and read back at once. No wait could tell whether another library is writing it at the same moment, so the rule is: the library that wrote `library.json` last owns the folder. The other device finds another library on its next pass and stops with the typed state. The two libraries never merge, because every head names its library and pull skips heads of another library.
- A device that wrote `library.json` and finds another library there before its creation held removes its own prefix, the only prefix it writes. The creation holds once the device finds its own `library.json` again after it has written its first head. Nothing is removed when the device did not write `library.json`, after the creation held, when `library.json` is missing or names this library, or when a head in the prefix was written by another writer session.
- A share sync failure reaches the UI as a typed state of that library.

### Storage primitives

- Required: whole-object GET, PUT and DELETE, and a one-level LIST.
- Used when a probe confirms them, with a fallback otherwise: conditional GET for polls, create-if-absent for `library.json`, If-Match on the device's own head. OpenDAL capabilities are static, so each conditional feature is probed by its effect on the device's own prefix. WebDAV through OpenDAL has no conditional writes and always takes the fallback.
- A write probe runs at setup and at each start; if it fails, share sync is off for that library and the state reaches Dart as a typed state.
- Not required: rename, append, multipart, locks, versioning, server time, recursive or consistent listing.

### Sizing

Estimates for a reference library of about 60 MB DB and 1,100 changes a day: about 75 MB and 200 objects in the share after a year, flat over time on servers that keep no old versions (a default Backblaze B2 bucket or Nextcloud versions keep overwritten objects); about 9 KB up per push; about 25 MB up for a monthly base cut, by the keeper only; about 75 MB down for a new device.

## 4. Library identity and records

### Identity

A library is identified by a stable `library_id`, never by its path. The path is a storage binding that can change, for example when an SD card is remounted or a folder is moved. A portable library also writes its descriptor to `.rune/library.json`, so a folder opened on another device is recognised by its `library_id`.

### Library records

The registry holds one record per library, as independent fields:

- `record_id`: the registry entry. `library_id`: the stable catalog identity and the library's sync namespace. A remote-control record has none.
- `alias`: the display name.
- Protocol: `Native` (this device owns the library), `RuneForward` (another Rune device owns it, as a replica or in remote control), or `SubsonicTranslate`.
- FS adapters: where audio bytes are read from: local folder, Android SAF, WebDAV, S3, or fetching from the owning Rune device.
- Catalog placement: where the catalog DB lives.
- Background sync, and a credential reference (never the secret).

One function validates a record when it is registered and when the registry is loaded, and returns a typed error that the UI shows. The add-library guide offers only legal combinations. Examples of illegal records: remote control with an FS adapter; a native record without one; a replica reading through WebDAV. A replica syncs the owner's database and fetches songs from the owner on demand; it never knows or uses the owner's storage.

The UI opens, registers and removes records by `record_id`. The last open record reopens on start. Removing a record never deletes media or the catalog DB.

**Catalog placement:**

- `InMediaRoot`: the DB in `.rune/` in the media folder (portable).
- `Redirected(key)`: the DB in the config directory, on this device only.
- `InShare(key)`: the working copy in the config directory, and the library's data kept in a writable WebDAV or S3 share (chapter 3).
- `ConfigReplica`: a replica of another Rune device's library in the config directory.
- `None`: no local DB, for a Subsonic library or a remote-control record.

An OpenDAL library never keeps its live DB in the bucket.

### Metadata scopes

- **Catalog**: tracks, albums, artists, genres, artwork, playlists, mixes, likes, statistics, fingerprints and similarity. It lives in the library DB and is replicated under the library's namespace.
- **Device-local library state**: paths, availability, the SAF cache, download rules and jobs, and the search index. It lives in the library DB, marked local, and is never replicated.
- **Session state**: queue, position and playback mode, in the node's session store. Handoff transfers it explicitly.
- **Node state**: the registry, trust, grants and credentials, in the config directory. Not replicated.

### Library pool

Each library DB is opened once and shared by the UI, sync and background work. A library nobody uses is closed after an idle timeout. Background work holds a library for its pass without counting as use, so a library that only background work touches is released and reopened by the next pass.

Sync addresses a library by its `library_id`. A device answers "not served" for a library it does not hold, and lists to another device only the libraries that device may read.

### Library tasks

Scan, analysis and deduplication keep one state per library on the node: a run id, progress and one outcome. The state changes under one lock and is sent in full to every device that shows the library, so every UI shows the same state.

- A library runs one task at a time. While one runs, starting another is refused with a typed error that names the running task. There is no queue, and no request stops another task.
- A cancel answers after the task has stopped.
- A failure to write the catalog fails the task. A file that cannot be decoded is skipped and tried again by a later run.

## 5. Access control

### Grants

Permissions are granted per remote device and checked on every request, not only when a connection opens. Pairing trust (`Pending`, `Approved`, `Blocked`) is a precondition: only an approved device is checked against grants.

- **Control** allows attaching as a controller: commands, status and queue edits. It includes **browse** on the libraries the player plays from, because a controller has to browse and search to add songs to the queue. Browse covers queries, cover art and metadata through the control connection. It never lets the device pull a replica, list the library or download audio.
- **Library access** per library, as two independent ordered levels:
    - connection: Not shared; View (browsing, searching, cover art, metadata and streaming through the device's connection); View and edit (also the device's changes to likes, playlists, mixes and statistics through that connection);
    - sync: None; Pull (pulling a one-way replica); Pull and push (two-way sync: the device's replica changes are accepted).
- The UI keeps this simple: one choice per library, Not shared (connection Not shared, sync None), Read only (View, Pull) or Read and write (View and edit, Pull and push). Any other combination is set through the API or the grants file in the config directory, and the UI shows it without changing it until the user picks one of the three.
- **Administer** covers pairing approval, connection settings and library maintenance. It exists in the API; the UI never grants it to another device.
- There is no temporary or per-cast access. Every grant is set beforehand on the Devices page and stays until changed.
- There is no separate permission for offline copies: streaming and downloading read the same bytes.
- A device with sync Pull pulls a one-way replica; its local changes are not accepted back, so the client hides write actions on a read-only replica. Liking while controlling writes the player's library through the connection, so a Guest without View and edit sees no like action.

Presets:

- **My device**: Control, plus Read and write on every library, including libraries created later.
- **Guest**: Control only.

Casting to B is controlling B, so this device needs Control on B. When the cast items come from this device's library, B also needs View on that library, set on this device's Devices page.

### Cast preflight

Before a cast the controller checks the player (reachable, accepts any granted library, supports load), access (Control on the player, View for the player on every library in the selection) and reachability (the player checks that it can reach each library's owner, because its network view can differ). The picker shows unusable players dimmed with the reason before the user picks one; a prompt appears only when access disappears between listing and loading. The load repeats every check.

### Enforcement

- Every library call from another device names its library. A call without a `library_id` is rejected; it is never served from the library open in the UI.
- Every call in the request catalogue has an access rule for calls from other devices. This-device calls (trust management, scrobbling sign-in, server control, logs, licences) are refused when they come from another device.
- A call whose body names libraries, through its items, also declares in the catalogue what they mean: the call acts on each of them, so each is authorized with the call's access rule and must be the library the call names; or the player reads them under its own grants (load, cast, handoff). The libraries are found from the body's types and checked where the call is authorized, before any handler runs.
- Short-lived tokens, issued only to approved devices after a signature challenge, authenticate the WebSocket, the file and media routes and sync.
- The capability snapshot sent to a controller marks every call its grants do not allow as unavailable.

### Policy boundary

The connection layer asks; the application decides. They meet at one trait:

```rust
pub trait AccessPolicy: Send + Sync + 'static {
    type Action: Send + Sync;
    type Denial: std::error::Error + Send + Sync;
    fn check(&self, peer: &PeerIdentity, action: &Self::Action) -> Result<(), Self::Denial>;
    fn changes(&self) -> tokio::sync::watch::Receiver<u64>;
}
```

- `PeerIdentity` is the fingerprint and key authenticated by TLS or the sync challenge; the transport never builds an action from anything a peer merely claims.
- The transport calls `check` on every HTTP route, every WebSocket message and every sync session. Long-lived access subscribes to `changes`, re-checks, and is cancelled when a grant drops: a session that loses Control is closed, and streams and sync sessions stop.
- Rune's actions are `Control`, `Browse { library_id }`, `LibraryRead { library_id }`, `LibraryWrite { library_id }` and `Administer`.
- Settings stored in Flutter are never reachable from another device. A controller sees the player and libraries only.

## 6. User interface

The UI uses the existing settings components (`SettingsTileTitle` rows, `ContentDialog` with `SubtitleButton` choices, `ComboBox`) and adds no new visual language. It never shows a grid of permissions:

- Presets cover most devices, so most people only see the approval dialog.
- Per-library settings live on an ordinary settings page.
- Missing access is shown where it matters, with the reason.

### Devices page

Three sections:

- This device: its name, "Visible to nearby devices" and "Allow other devices to connect".
- Devices that can connect to this one: name and a short summary of what the device may do. Only "Waiting for approval" and "Blocked" carry a coloured badge. A device that is controlling this one can be disconnected from its row.
- Other devices: search nearby and add manually.

The approval dialog offers My device, Guest and Block. The device page, opened from "Customize", has a "Control playback" switch and one Not shared / Read only / Read and write choice per library.

### Adding a library

One guide:

1. Where the music is: local folder, another Rune device, WebDAV, S3, or a Subsonic server.
2. Rune detects what is there (an existing library, data kept in a share).
3. The next step depends on the source:
    - a local folder: where the library's data lives, on this device or in the media folder;
    - a WebDAV or S3 folder: a folder that already holds a library's data is joined; a writable empty folder asks whether the library's data is kept in the shared folder or on this device, as two choices with no default; a read-only folder keeps the data on this device;
    - another Rune device or a Subsonic server: play on this device, or control that device.

The library list marks replicas with the device they come from and "Read only", and has a background-sync switch per library. On a read-only replica the like button and playlist and mix editing are hidden.

### Choosing where to play

Which library is browsed and where the sound comes out stay independent:

- The library switcher shows one library at a time: a local library, a replica, a Subsonic library or an @RR entry.
- "Play on" sits among the playback controls. Its dialog lists this device and the online players, with unusable ones dimmed and the reason shown. Picking one hands playback off there; the player bar and queue then show and control that player while browsing continues. Picking this device brings playback back. While another device plays, the button is highlighted and its tooltip names the device.
- Track and collection menus have "Play on" too. A cast track replaces the target's queue or plays next, and casting switches the player bar to that device or not, as two playback settings say.
- The queue panel has a handoff button that opens the same dialog.
- A player that cannot play every library in the queue stays dimmed with the reason, and the handoff is refused; nothing is filtered out.

### Wording

In every locale: "Sharing" instead of "Server"; "Allow other devices to connect"; "Visible to nearby devices"; My device, Guest, Waiting for approval and Blocked. The words grant, token and permission do not appear. Administer has no UI.

## 7. Acceptance scenarios

Each scenario is covered by an automated test that drives the real components across their real boundaries and asserts what the user sees. A test counts only if it fails when the behaviour it covers is broken.

- A controls B while B plays a library hosted by C; revoking B's grant on C stops playback cleanly, and the item is marked unavailable with a typed reason.
- A casts an item from A's library to B, which holds View on that library; without that grant B is dimmed in A's device list and the load is refused.
- Preflight reports "not possible" with the reason for a library B cannot read; the load still fails if a grant is revoked after preflight.
- A controller renders the visualizer for a player on another Rune device, with the clock running while the player plays, and the chart and spectrum received through the feed.
- A Subsonic server that lists `runeVisualizerFeed` enables the visualizer for its jukebox; one that does not, or lacks `openSubsonic: true`, keeps it disabled.
- A Subsonic jukebox that plays shows as playing in the play bar, the OS media controls and the visualizer, and the play bar shows the jukebox's own track, not values sent by the feed.
- Opening a Subsonic library in remote-control mode while this device plays pauses this device's player; from then on the play bar, queue and spectrum show only the jukebox, and opening a local library again shows this device's session paused where it stopped.
- Handoff from a local session to B and back preserves the queue and position across libraries.
- The local UI keeps browsing and playing a local library while it controls another device's player.
- A Subsonic jukebox rejects items from other libraries before any HTTP call.
- With a Subsonic library open, the cover wall and the playing cover show that library's covers, fetched with `getCoverArt` and cached on this device.
- Two controllers edit one queue concurrently; the edit against a stale revision is rejected, and both see the same queue afterwards.
- A source that feeds the spectrum only offers only spectrum scenes, and a source without a feed hides the visualizer.
- An OpenDAL library on a read-only service scans and plays with its DB in the config directory, and nothing is written to the bucket.
- A library on a writable share keeps its data in the share; a second device joins from it and catches up, and a half-written publish is ignored.
- A device that joins a share and changes something before it has published anything still gets that change to every device.
- A remote controller receives live status updates from a headless server.
- A portable library opened on a second device is recognised by `library_id`.
- An idle library DB is released and reopened by an authorised sync request without losing worker state.
- A device with sync Pull on a library pulls a replica; its local likes are not accepted back, and the UI hides them.
- A Guest controlling a player can queue songs but sees no like action.
- The device page offers one Not shared / Read only / Read and write choice per library and sets both levels from it; a combination set through the API is shown unchanged.
- A forwarded `UpdateClientStatusRequest` from another device is refused.
- An endpoint URL with an unknown parameter is rejected with that parameter named; an unknown `opt-` parameter is ignored; a newer `v` asks for a newer version.
- While a scan runs, starting analysis on the same library from any device is refused with the running task named, and every device shows the scan's progress.
