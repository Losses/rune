#!/usr/bin/env python3
"""Inspect an actual debug Rune window through its VM service.

Use an SSH tunnel for a remote service. --set-percent only operates on the
explicit scale-validation profile. This measures the live renderer's metrics;
it does not simulate native mouse, IME, drop or monitor-transition events.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import time

spec = importlib.util.spec_from_file_location(
    "vm_eval", Path(__file__).parent / "android" / "vm_eval.py"
)
vm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("service", help="VM service URL, through a tunnel if remote")
    parser.add_argument("--set-percent", type=int, choices=[75, 100, 125, 150, 175, 200])
    args = parser.parse_args()
    connection = vm.ws_connect(args.service.removeprefix("http://").rstrip("/"))
    connection.settimeout(20)
    try:
        def rpc(method, **params):
            reply = vm.rpc(connection, method, params)
            if "error" in reply:
                raise RuntimeError(reply["error"])
            return reply["result"]

        isolates = rpc("getVM")["isolates"]
        isolate = next(item for item in isolates if item["name"] == "main")["id"]
        libraries = rpc("getIsolate", isolateId=isolate)["libraries"]
        library = next(item for item in libraries if item["uri"] == "package:rune/main.dart")["id"]

        def evaluate(expression):
            value = rpc("evaluate", isolateId=isolate, targetId=library, expression=expression)
            if value.get("type") in ("Error", "@Error"):
                raise RuntimeError(value)
            return value.get("valueAsString")

        context = "rootNavigatorKey.currentContext!"
        provider = f"Provider.of<AppScaleProvider>({context}, listen: false)"
        if args.set_percent is not None:
            key = evaluate("StorageKeyManager.getStorageKey('interface_scale')")
            if key != "ci.not.rune#scale-validation:interface_scale":
                raise RuntimeError("Refusing to change an application outside the scale-validation profile")
            evaluate(f"{provider}.setPercentage({args.set_percent})")
            deadline = time.monotonic() + 10
            while evaluate(f"{provider}.percentage.toString()") != str(args.set_percent):
                if time.monotonic() >= deadline:
                    raise TimeoutError("Scale did not apply")
                time.sleep(0.1)
            time.sleep(0.5)  # layout and the visualizer's resize debounce

        raw = evaluate("(() { "
            f"final c = {context}; final p = {provider}; "
            "final m = MediaQuery.of(c); final v = View.of(c); "
            "return [p.percentage, m.size.width, m.size.height, m.devicePixelRatio, "
            "v.physicalSize.width, v.physicalSize.height, v.devicePixelRatio].join(','); })()")
        fields = [float(value) for value in raw.split(",")]
        names = ["percentage", "ui_width", "ui_height", "effective_dpr",
                 "physical_width", "physical_height", "system_dpr"]
        result = dict(zip(names, fields, strict=True))
        expected_dpr = result["system_dpr"] * result["percentage"] / 100
        if abs(result["effective_dpr"] - expected_dpr) > 0.000001:
            raise AssertionError(f"Incorrect effective density: {result}")
        for axis in ["width", "height"]:
            if abs(result[f"ui_{axis}"] * result["effective_dpr"] - result[f"physical_{axis}"]) > 1:
                raise AssertionError(f"Inconsistent {axis} metrics: {result}")
        print(json.dumps(result))
    finally:
        connection.close()


if __name__ == "__main__":
    main()
