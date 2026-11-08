#!/usr/bin/env python3
"""Generate the Rinf-visible typed capability messages from the request catalogue."""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[2]
CATALOGUE = ROOT / "native/requests/src/lib.rs"
OUTPUT = ROOT / "native/hub/src/messages/request_capabilities.rs"


def field_name(request: str) -> str:
    return "".join(
        ("_" if index and char.isupper() else "") + char.lower()
        for index, char in enumerate(request)
    )


def main() -> None:
    source = CATALOGUE.read_text()
    requests = list(dict.fromkeys(re.findall(r'request: "([A-Za-z0-9]+Request)"\.to_string\(\)', source)))
    if not requests:
        raise RuntimeError(f"No requests found in catalogue: {CATALOGUE}")

    fields = [(field_name(request), request) for request in requests]
    lines = [
        "// Generated from the request catalogue by native/requests/generate_capabilities.py.",
        "use rinf::{RustSignal, SignalPiece};",
        "use serde::{Deserialize, Serialize};",
        "",
        "#[cfg(test)]",
        "use crate::rpc::RpcTarget;",
        "use crate::rpc::{Slot, UnsupportedReason};",
        "",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]",
        "pub enum RequestCapability {",
        "    Available,",
        "    Hidden,",
        "    Disabled { reason: CapabilityReason },",
        "}",
        "",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]",
        "pub enum CapabilityReason {",
        "    NotImplemented,",
        "    OwnerOnly,",
        "}",
        "",
        "#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RustSignal)]",
        "pub struct RequestCapabilities {",
    ]
    lines.extend(f"    pub {field}: RequestCapability," for field, _ in fields)
    lines.extend(
        [
            "}",
            "",
            "impl RequestCapabilities {",
            "    #[cfg(test)]",
            "    pub(crate) fn all_available() -> Self {",
            "        Self {",
        ]
    )
    lines.extend(f"            {field}: RequestCapability::Available," for field, _ in fields)
    lines.extend(
        [
            "        }",
            "    }",
            "",
            "    #[cfg(test)]",
            "    pub(crate) fn from_target(target: &RpcTarget) -> Self {",
            "        Self {",
        ]
    )
    lines.extend(
        f"            {field}: target.capability::<crate::messages::{request}>(),"
        for field, request in fields
    )
    lines.extend(
        [
            "        }",
            "    }",
            "}",
            "",
            "impl From<Slot> for RequestCapability {",
            "    fn from(slot: Slot) -> Self {",
            "        match slot {",
            "            Slot::Native | Slot::Forward | Slot::Translate => Self::Available,",
            "            Slot::Unsupported { reason: UnsupportedReason::NotImplemented } => Self::Hidden,",
            "            Slot::Unsupported { reason: UnsupportedReason::OwnerOnly } => {",
            "                Self::Disabled { reason: CapabilityReason::OwnerOnly }",
            "            }",
            "        }",
            "    }",
            "}",
            "",
            "crate::implement_rinf_rust_signal_trait!(RequestCapabilities);",
            "",
        ]
    )
    OUTPUT.write_text("\n".join(lines))


if __name__ == "__main__":
    main()
