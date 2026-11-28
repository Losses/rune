"""Confirm the disposable fixture folder in Android's DocumentsUI picker."""

import argparse
import re
import subprocess
import time
import xml.etree.ElementTree as ET


def focused_package():
    # Query the window manager without attaching UI automation to the app.
    # A dialog's window title may omit its component; its focused activity
    # still identifies the package.
    result = subprocess.run(
        ["adb", "shell", "dumpsys window displays | grep -E 'mCurrentFocus=|mFocusedApp='"],
        capture_output=True, text=True,
    )
    for line in result.stdout.splitlines():
        component = re.search(r"\bu\d+\s+([\w.]+)/", line)
        if component:
            return component.group(1)
    return None


def confirm_folder(folder, app_package):
    selected = False
    confirmed = False
    while True:
        # The host stops this helper when flutter test exits, including on a
        # build failure. A device may return to Rune without a second prompt.
        package = focused_package()
        if selected and package == app_package:
            return
        if not package or not package.endswith(".documentsui"):
            time.sleep(0.5)
            continue
        result = subprocess.run(
            ["adb", "shell", "uiautomator dump /data/local/tmp/rune-picker.xml"
             " >/dev/null && cat /data/local/tmp/rune-picker.xml"],
            capture_output=True,
        )
        try:
            nodes = list(ET.fromstring(result.stdout).iter("node"))
        except ET.ParseError:
            nodes = []  # The activity may be changing.
        nodes = [n for n in nodes if n.get("package", "").endswith(".documentsui")]
        if selected:
            # Both screens can use android:id/button1. Wait for the dialog.
            ready = not confirmed and any(n.get("resource-id") == "android:id/button2" for n in nodes)
        else:
            ready = any(n.get("text") == folder for n in nodes)
        if ready:
            for node in nodes:
                if node.get("resource-id", "").split("/")[-1] not in ("button_select", "button1"):
                    continue
                if node.get("enabled") != "true":
                    continue
                x1, y1, x2, y2 = map(int, re.findall(r"\d+", node.attrib["bounds"]))
                subprocess.run(
                    ["adb", "shell", "input", "tap", str((x1 + x2) // 2),
                     str((y1 + y2) // 2)], check=True,
                )
                confirmed = selected
                selected = True
                break
        time.sleep(0.5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folders", nargs="+", help="fixture folders in picker order")
    parser.add_argument("--count", type=int, default=1)
    parser.add_argument("--package", default="ci.not.rune")
    args = parser.parse_args()
    for _ in range(args.count):
        for index, folder in enumerate(args.folders, start=1):
            confirm_folder(folder, args.package)
            print(f"Confirmed DocumentsUI selection {index}/{len(args.folders)}", flush=True)
