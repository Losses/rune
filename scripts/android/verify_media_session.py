#!/usr/bin/env python3
"""Check Rune's system media controls on an isolated Android test device.

Start the first of two long tracks in Rune's sequential queue. Pass both
titles in queue order and supply the artist and album of each track.
Keep other media apps closed because Android dispatches these buttons globally.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
from collections.abc import Callable
from dataclasses import dataclass

BUTTON_TARGET = "Media button session is "
PACKAGE = "ci.not.rune"
RUNE_SESSION = re.compile(
    r"^[ \t]*PlaybackService (ci\.not\.rune/PlaybackService(?:/\S+)? \(userId=\d+\))$"
)
PLAYBACK_STATE = re.compile(
    r"state=PlaybackState \{state=(?:[A-Z_]+\()?([0-9]+)\)?, "
    r"position=(-?\d+), buffered position=-?\d+, speed=([-+0-9.eE]+)"
)
DESCRIPTION = re.compile(r"metadata: .*description=(.*)")
STOPPED_LOG = "playback::internal: Playback stopped"


@dataclass(frozen=True)
class Session:
    handle: str
    receives_buttons: bool
    state: int | None
    position: int | None
    speed: float | None
    description: str | None

    def playing(self, description: str) -> bool:
        return (
            self.state == 3
            and self.speed is not None
            and self.speed > 0
            and self.description == description
        )

    def paused(self, description: str) -> bool:
        return self.state == 2 and self.speed == 0 and self.description == description


def parse_sessions(dump: str) -> list[Session]:
    lines = dump.splitlines()
    targets = {
        line.strip().removeprefix(BUTTON_TARGET)
        for line in lines
        if line.lstrip().startswith(BUTTON_TARGET)
    }
    if not targets:
        raise RuntimeError("Unrecognized Android media-session dump")

    sessions = []
    for index, line in enumerate(lines):
        header = RUNE_SESSION.fullmatch(line)
        if not header:
            continue
        indent = len(line) - len(line.lstrip())
        fields = []
        for field in lines[index + 1:]:
            if len(field) - len(field.lstrip()) <= indent:
                break
            fields.append(field)
        fields = "\n".join(fields)
        state = PLAYBACK_STATE.search(fields)
        description = DESCRIPTION.search(fields)
        handle = header[1]
        sessions.append(Session(
            handle, handle in targets,
            int(state[1]) if state else None,
            int(state[2]) if state else None,
            float(state[3]) if state else None,
            description[1] if description else None,
        ))
    return sessions


class Device:
    def __init__(self, serial: str | None) -> None:
        self.adb = ["adb", "-s", serial] if serial else ["adb"]

    def shell(self, *args: str) -> str:
        return subprocess.run(
            [*self.adb, "shell", *args],
            check=True, capture_output=True, text=True, timeout=5,
        ).stdout

    def sessions(self) -> list[Session]:
        return parse_sessions(self.shell("dumpsys", "media_session"))

    def dispatch(self, command: str, handle: str) -> None:
        sessions = self.sessions()
        if (
            len(sessions) != 1
            or sessions[0].handle != handle
            or not sessions[0].receives_buttons
        ):
            raise RuntimeError(f"Rune is not the media button target for {command}: {sessions}")
        self.shell("cmd", "media_session", "dispatch", command)

    def stop_events(self, pid: str) -> set[str]:
        log = self.shell(
            "logcat", "-d", "-b", "main", f"--pid={pid}",
            "-v", "threadtime", "-v", "usec", "-s", "playback:I",
        )
        return {line for line in log.splitlines() if line.endswith(STOPPED_LOG)}

    def stop(self, handle: str) -> None:
        pid = self.shell("pidof", PACKAGE).strip()
        if not re.fullmatch(r"[1-9][0-9]*", pid):
            raise RuntimeError(f"Expected one Rune process, observed {pid!r}")
        # A released MediaSession alone can hide a player that kept running.
        earlier = self.stop_events(pid)
        self.dispatch("stop", handle)
        self.wait_for(
            "Rune stopped native playback and released its media session after Stop",
            lambda sessions: not sessions and bool(self.stop_events(pid) - earlier),
        )

    def wait_for(
        self, expected: str, condition: Callable[[list[Session]], bool]
    ) -> list[Session]:
        deadline = time.monotonic() + 10
        while True:
            sessions = self.sessions()
            if condition(sessions):
                print(f"{expected}: {sessions}", flush=True)
                return sessions
            if time.monotonic() >= deadline:
                raise RuntimeError(f"Expected {expected}; observed {sessions}")
            time.sleep(0.2)

    def hold(self, expected: str, condition: Callable[[list[Session]], bool]) -> None:
        # Bracket the hold interval between two ADB queries so a slow response
        # cannot count as time spent holding the state.
        deadline = None
        while True:
            probe_started = time.monotonic()
            sessions = self.sessions()
            if not condition(sessions):
                raise RuntimeError(f"Expected {expected}; observed {sessions}")
            if deadline is None:
                deadline = time.monotonic() + 1
            elif probe_started >= deadline:
                print(f"{expected}: {sessions}", flush=True)
                return
            time.sleep(0.2)


def verify(device: Device, first: str, second: str) -> None:
    sessions = device.wait_for(
        "Rune as the media button target",
        lambda current: len(current) == 1 and current[0].receives_buttons,
    )
    handle = sessions[0].handle
    try:
        verify_controls(device, handle, first, second)
    except (OSError, RuntimeError, subprocess.SubprocessError):
        try:
            # A different app or a replacement session can take the media keys.
            sessions = device.sessions()
            if (
                len(sessions) == 1
                and sessions[0].handle == handle
                and sessions[0].receives_buttons
            ):
                device.dispatch("stop", handle)
                device.wait_for(
                    "Rune released the test session after failure",
                    lambda current: all(session.handle != handle for session in current),
                )
        except (OSError, RuntimeError, subprocess.SubprocessError) as error:
            print(f"Could not stop Rune after verification failed: {error}", file=sys.stderr)
        raise


def verify_controls(device: Device, handle: str, first: str, second: str) -> None:
    def routed(predicate: Callable[[Session], bool]) -> Callable[[list[Session]], bool]:
        return lambda sessions: (
            len(sessions) == 1
            and sessions[0].handle == handle
            and sessions[0].receives_buttons
            and predicate(sessions[0])
        )

    playing_first = routed(lambda session: session.playing(first))
    device.wait_for(f"Rune playing {first!r}", playing_first)
    device.hold(f"{first!r} still playing", playing_first)
    device.dispatch("pause", handle)
    paused = device.wait_for(
        f"Rune paused on {first!r} after playback advanced",
        routed(lambda session: session.paused(first) and session.position > 0),
    )[0]
    device.hold(
        f"pause position held at {paused.position} ms",
        routed(lambda session: session.paused(first) and session.position == paused.position),
    )

    device.dispatch("play", handle)
    device.wait_for(
        f"Rune resumed {first!r} from the pause position",
        routed(lambda session: session.playing(first) and session.position >= paused.position),
    )
    device.hold(f"{first!r} still playing", playing_first)
    device.dispatch("pause", handle)
    device.wait_for(
        "Rune paused at a later position after resuming",
        routed(lambda session: session.paused(first) and session.position > paused.position),
    )
    device.dispatch("play", handle)
    device.wait_for(f"Rune playing {first!r} again", playing_first)

    device.dispatch("next", handle)
    device.wait_for(
        f"Rune playing {second!r} after Next", routed(lambda session: session.playing(second))
    )
    device.dispatch("previous", handle)
    device.wait_for(f"Rune playing {first!r} after Previous", playing_first)
    device.stop(handle)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", help="adb serial. Defaults to the current adb device.")
    parser.add_argument("--first-metadata", required=True, nargs=2, metavar=("ARTIST", "ALBUM"))
    parser.add_argument("--second-metadata", required=True, nargs=2, metavar=("ARTIST", "ALBUM"))
    parser.add_argument("first_title")
    parser.add_argument("second_title")
    args = parser.parse_args()
    first = (args.first_title, *args.first_metadata)
    second = (args.second_title, *args.second_metadata)
    if not all(field.strip() for field in (*first, *second)):
        parser.error("Supply a nonempty title, artist and album for each track.")
    # Android dumps one combined description. Match it as a whole because
    # each metadata field can contain commas.
    first_description = ", ".join(first)
    second_description = ", ".join(second)
    if first_description == second_description:
        parser.error("Supply tracks with distinct Android media descriptions.")
    try:
        verify(Device(args.serial), first_description, second_description)
    except subprocess.CalledProcessError as error:
        print(error.stderr.strip() or str(error), file=sys.stderr)
        return 1
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
