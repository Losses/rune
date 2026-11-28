#!/usr/bin/env python3
"""Check Rune's system media controls on a connected Android device.

Start the first of two tracks with distinct titles in Rune's sequential queue.
Use tracks long enough to keep playing during the test. Pass both titles in
queue order. The test sends Android media buttons and ends by stopping Rune.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
from dataclasses import dataclass

BUTTON_TARGET = "Media button session is "
RUNE_SESSION = re.compile(
    r"^[ \t]*PlaybackService (ci\.not\.rune/PlaybackService(?:/\S+)? \(userId=\d+\))$"
)
PLAYBACK_STATE = re.compile(
    r"state=PlaybackState \{state=(?:[A-Z_]+\()?([0-9]+)\)?, "
    r"position=(-?\d+), buffered position=-?\d+, speed=([-+0-9.eE]+)"
)
DESCRIPTION = re.compile(r"metadata: .*description=(.*)")


@dataclass(frozen=True)
class Session:
    handle: str
    receives_buttons: bool
    state: int | None
    position: int | None
    speed: float | None
    description: str | None

    def shows(self, title):
        return self.description is not None and (
            self.description == title or self.description.startswith(f"{title}, ")
        )

    def playing(self, title):
        return self.state == 3 and self.speed > 0 and self.shows(title)

    def paused(self, title):
        return self.state == 2 and self.speed == 0 and self.shows(title)


def parse_sessions(dump):
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
    def __init__(self, serial):
        self.adb = ["adb", "-s", serial] if serial else ["adb"]

    def shell(self, *args):
        return subprocess.run(
            [*self.adb, "shell", *args],
            check=True, capture_output=True, text=True, timeout=5,
        ).stdout

    def sessions(self):
        return parse_sessions(self.shell("dumpsys", "media_session"))

    def dispatch(self, command):
        sessions = self.sessions()
        if len(sessions) != 1 or not sessions[0].receives_buttons:
            raise RuntimeError(f"Rune is not the media button target for {command}: {sessions}")
        self.shell("cmd", "media_session", "dispatch", command)

    def wait_for(self, expected, condition):
        deadline = time.monotonic() + 10
        while True:
            sessions = self.sessions()
            if condition(sessions):
                print(f"{expected}: {sessions}", flush=True)
                return sessions
            if time.monotonic() >= deadline:
                raise RuntimeError(f"Expected {expected}; observed {sessions}")
            time.sleep(0.2)

    def hold(self, expected, condition):
        # Check the state over an interval so a single pause report cannot hide moving progress.
        deadline = time.monotonic() + 1
        while True:
            sessions = self.sessions()
            if not condition(sessions):
                raise RuntimeError(f"Expected {expected}; observed {sessions}")
            if time.monotonic() >= deadline:
                print(f"{expected}: {sessions}", flush=True)
                return
            time.sleep(0.2)


def routed(predicate):
    return lambda sessions: (
        len(sessions) == 1 and sessions[0].receives_buttons and predicate(sessions[0])
    )


def verify(device, first, second):
    playing_first = routed(lambda session: session.playing(first))
    device.wait_for(f"Rune playing {first!r}", playing_first)
    device.hold(f"{first!r} still playing", playing_first)
    device.dispatch("pause")
    paused = device.wait_for(
        f"Rune paused on {first!r} after playback advanced",
        routed(lambda session: session.paused(first) and session.position > 0),
    )[0]
    device.hold(
        f"pause position held at {paused.position} ms",
        routed(lambda session: session.paused(first) and session.position == paused.position),
    )

    device.dispatch("play")
    device.wait_for(
        f"Rune resumed {first!r} from the pause position",
        routed(lambda session: session.playing(first) and session.position >= paused.position),
    )
    device.hold(f"{first!r} still playing", playing_first)
    device.dispatch("pause")
    device.wait_for(
        "Rune paused at a later position after resuming",
        routed(lambda session: session.paused(first) and session.position > paused.position),
    )
    device.dispatch("play")
    device.wait_for(f"Rune playing {first!r} again", playing_first)

    device.dispatch("next")
    device.wait_for(
        f"Rune playing {second!r} after Next", routed(lambda session: session.playing(second))
    )
    device.dispatch("previous")
    device.wait_for(f"Rune playing {first!r} after Previous", playing_first)
    device.dispatch("stop")
    device.wait_for("Rune released its media session after Stop", lambda sessions: not sessions)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", help="adb serial. Defaults to the current adb device.")
    parser.add_argument("first_title")
    parser.add_argument("second_title")
    args = parser.parse_args()
    if not args.first_title or not args.second_title or args.first_title == args.second_title:
        parser.error("Supply two distinct nonempty track titles.")
    try:
        verify(Device(args.serial), args.first_title, args.second_title)
    except subprocess.CalledProcessError as error:
        print(error.stderr.strip() or str(error), file=sys.stderr)
        return 1
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
