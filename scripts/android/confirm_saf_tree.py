"""Confirm the disposable fixture folder in Android's DocumentsUI picker."""

import re
import subprocess
import sys
import time
import xml.etree.ElementTree as ET


def confirm_folder(folder):
    selected = False
    while True:
        # The host stops this helper when flutter test exits, including on a
        # build failure or if DocumentsUI returns without a second prompt.
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
            ready = any(n.get("resource-id") == "android:id/button2" for n in nodes)
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
                if selected:
                    return
                selected = True
                break
        time.sleep(0.5)


if __name__ == "__main__":
    confirm_folder(sys.argv[1])
