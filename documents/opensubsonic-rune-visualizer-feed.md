# runeVisualizerFeed OpenSubsonic extension

## 1. Overview

runeVisualizerFeed gives a client what it needs to draw a visualizer for a server's jukebox: the jukebox's playback clock, a per-track chart, cover images and the realtime spectrum.

The feed is read-only. It does not control the jukebox; jukebox control stays with the standard `jukeboxControl` endpoint. The messages a client sends on the feed only start, stop or configure delivery.

## 2. Declaration and support detection

A server declares the extension in the `openSubsonicExtensions` array returned by `getOpenSubsonicExtensions`, listing every version it supports:

    {
      "name": "runeVisualizerFeed",
      "versions": [1]
    }

The server must identify itself as OpenSubsonic by returning `"openSubsonic": true` in its responses.

A client probes `ping` first. Only when that response has `openSubsonic: true` does it request `getOpenSubsonicExtensions`. It uses the extension when the declaration lists a version it implements, and then uses the highest such version. Version 1 is the only version defined so far. A missing declaration, a declaration without a version the client implements, or a server without `openSubsonic: true` means the client draws no visualizer for that jukebox.

Rune probes when the library is added, when its credential is replaced, and each time it is opened. This declaration says nothing about any other Rune extension.

## 3. Version 1 endpoint

The endpoint is `getRuneVisualizerFeed`, called as a Subsonic REST GET:

    GET <server-base>/rest/getRuneVisualizerFeed.view

It has no parameters of its own. The client sends the usual Subsonic parameters and credentials. A successful response carries the feed URL in `subsonic-response.runeVisualizerFeed.url`:

    {
      "subsonic-response": {
        "status": "ok",
        "version": "1.16.1",
        "runeVisualizerFeed": {
          "url": "/feed"
        }
      }
    }

The URL may be absolute or relative to the server base URL. The client resolves it against the base URL and changes `http` to `ws` and `https` to `wss`; `ws` and `wss` are used as given. Any other resulting scheme is an error.

Authentication is the same as for every Subsonic call: `u`, `t` and `s` for a password (`s` a random salt, `t` the lowercase hexadecimal MD5 of the password followed by the salt), or `apiKey`. The client also sends `c`, `v` and `f`. It adds these credentials to the resolved WebSocket URL as query parameters; a query key the server already put in the URL is kept as the server gave it.

A failure is a normal Subsonic failure: `status: "failed"` with an `error` object holding a numeric `code` and a string `message`. An HTTP or transport failure, malformed JSON, a malformed envelope or a missing `runeVisualizerFeed.url` is also a failure, and the client then draws no visualizer for that jukebox.

## 4. WebSocket feed

### Handshake

The client opens a WebSocket to the resolved URL. The credentials are in the URL query; the client sends no subprotocol and no custom header. The server authenticates the handshake with those credentials and applies the account's permissions.

### Frames

Every feed message is one binary WebSocket message:

- byte 0: the length N of the message name in bytes (one unsigned byte);
- bytes 1 to N: the message name in UTF-8;
- the bytes after the name, up to the last 16: the message body;
- the last 16 bytes: a UUID chosen by the sender. Receivers ignore it.

The client ignores text messages, frames it cannot split this way, bodies it cannot decode and message names it does not know.

### Body encoding

Bodies use bincode 1's `bincode::serialize`, which Rinf (the Rust–Dart message layer Rune uses) re-exports: fixed-width little-endian integers and `u64` lengths, not bincode's variable-length integer option. A body is the message's fields in the order listed below, with no field names and no padding:

- `bool`: one byte, 0 or 1.
- `u32`, `i32`, `f32`: four bytes, little-endian; `u64`, `f64`: eight bytes, little-endian.
- `String`: a `u64` byte count, then the UTF-8 bytes. `Vec<T>`: a `u64` element count, then the elements.
- `Option<T>`: one byte, 0 for none, or 1 followed by the value.
- An enum: the variant's index as a `u32` (counting from 0 in the order listed), then that variant's fields.

### Messages the client sends

After the connection opens, the client sends:

1. `SubscribeVisualizerFeedRequest`, with an empty body.
2. `SetRealtimeFFTEnabledRequest`: `enabled: bool` (true), `call_id: Option<u64>` (none).

When the client stops polling the jukebox or closes the library, it sends `UnsubscribeVisualizerFeedRequest` with an empty body and closes the WebSocket. The client does not wait for replies to any of these.

### Messages the server sends

`PlaybackStatus` is the jukebox's playback clock:

- `session_id: String`
- `state: PlaybackState`, an enum: `Playing`, `Paused`, `Stopped` (no fields)
- `progress_seconds: f32`: the position in the current track
- `progress_percentage: f32`
- `artist: Option<String>`, `album: Option<String>`, `title: Option<String>`
- `duration: f64`
- `index: Option<i32>`
- `item: Option<String>`
- `item_request: Option<PlayingItemRef>`: the current track
- `playback_mode: u32`
- `ready: bool`
- `cover_art_path: Option<String>`
- `lib_path: String`

The client reads only `state`, `progress_seconds` and `item_request`; the other fields exist because the message is Rune's own player status, and a server fills them as follows: `session_id` any string, `progress_percentage` the position divided by the duration (0 when the duration is unknown), `artist`, `album`, `title` and `duration` from the track (none and 0 when unknown), `index` the track's position in the jukebox playlist, `item` none, `playback_mode` 0, `ready` true, `cover_art_path` none, `lib_path` empty.

`PlayingItemRef` is an enum with two variants: `LibraryTrack { library_id: String, track: String }` and `IndependentFile { path: String }`. A Subsonic server sends `LibraryTrack` with `track` set to the Subsonic song id, as in the jukebox playlist, and `library_id` set to any string that stays the same for the whole connection. The client compares `track` with the current song it gets from `jukeboxControl`, and uses the whole value to match charts and covers to the current track.

`VisualizerFeedChart` carries a chart of the current track:

- `item: PlayingItemRef`
- `chart: Vec<u8>`

The chart is optional. Its bytes are the chart format of Rune's own visualizer analysis; this extension carries them without defining them, so a server that does not run Rune's analysis does not send this message. Without a chart the client still draws from the clock, the spectrum and the covers.

`VisualizerFeedCovers` carries cover images:

- `item: PlayingItemRef`
- `playing: Option<Vec<u8>>`: the current track's cover
- `random: Vec<Vec<u8>>`: other covers for the cover wall

Each image is PNG or JPEG. `random` holds at most 63 images, and all images in one message together take at most 16 MiB. The client ignores a message that exceeds either limit.

`RealtimeFFT` carries one spectrum frame:

- `value: Vec<f32>`: amplitudes scaled so the largest is 1

Rune sends 512 values per frame (all bins of a 512-point FFT of the mono mix) as the jukebox plays. The client ignores an empty frame and one with more than 4096 values.

### Order

Chart and cover messages belong to the track in their `item`. The client keeps them only while that track is the one in the latest `PlaybackStatus`, so a server sends the status for a new track before that track's chart and covers. When the status names a different track, the client drops the previous track's chart and covers. Charts and covers may arrive at any time after the status, in any order. Spectrum frames are independent of the other messages.

### Playback clock

The status has no timestamp. The client takes the time it receives each `PlaybackStatus` as the time of `progress_seconds`. While `state` is `Playing`, it advances the position from there on its own clock; otherwise it shows the reported position. While playing, a difference of more than 0.3 seconds between the reported and the shown position, or a new track, makes the client jump to the reported position; a smaller difference is pulled in gradually.

The server sends a status whenever the state, the track or the position (a seek) changes, and repeatedly while a track is loaded. Rune's own player reports every 100 ms, playing or paused. A server may send them less often; the client's clock then runs freely between them.

### Closing

The client closes the feed as described above. When the server closes the connection or it fails, the client stops reading and does not reconnect; the visualizer gets no new input until the jukebox is opened again.

## 5. Versions

A server lists every version it supports in `versions`; a client uses the highest version it implements. The version 1 layout of every message above is fixed: any change to a message's fields, their order, their meaning or the frame layout is a new version number, never a change to version 1.

## 6. Security

The credentials travel in the URL query, as in every Subsonic call. Server and client keep authenticated URLs and query strings out of application, proxy, access and diagnostic logs, and redact `apiKey`, `t` and `s` (and `p`, should a nonconforming client send one). A server reached over HTTPS serves the feed over `wss`.
