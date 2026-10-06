# Windows CI diagnostics

## Confirmed incident

Run https://github.com/Losses/rune/actions/runs/37535036270 (commit f0f936299acdad450a58a15f364ca8a5e617c522) was cancelled on 2026-10-06. Archived logs, not the incomplete live view, establish:

- x64: Cargo finished in 17m20s at 22:08:26 UTC; `[4a]` began at 22:08:27, then no Flutter output until cancellation at 22:57.
- ARM64: Cargo finished in 13m58s at 22:05:03 UTC; `[4a]` began at 22:05:04, then no Flutter output until cancellation at 22:58.
- Both produced hub.dll. The stalled command was `flutter --suppress-analytics --version`, not clean-path compilation or project pub resolution.

The pinned Flutter 3.47.6 startup script silently retries opening `bin/cache/flutter.bat.lock`. Running an SDK directly from the Nix store makes cache permissions worth checking, but this is a hypothesis, not a proven cause. Do not delete locks or change store ACLs on speculation. Source: https://github.com/flutter/flutter/blob/3.47.6/bin/internal/shared.bat

## Next diagnostic run

After the diagnostic changes are reviewed and available on GitHub, dispatch **Windows Test Build (x64 & ARM64)** with `arch=x64` first. Re-running the old run uses the old code and does not add monitoring. Keep toolchain versions, optimization and parallelism unchanged.

The parent monitor emits periodic heartbeat independently of command output, forwards bounded chunks from separate stdout/stderr files, and preserves full logs in the diagnostic artifact. Cross-stream ordering is approximate. Process snapshots include descendant command lines, CPU deltas, working set, free RAM and disk capacity. Flutter preflight records the resolved SDK, cache attributes/ACLs and a unique-file write probe; it does not alter existing locks.

Use process snapshots to distinguish sustained rustc/link CPU work, memory pressure, a silent Flutter batch process, or a waiting Dart/PowerShell child. A failed write probe supports a permissions issue; successful directory writes do not prove an existing lock file is writable or unlocked.

Allow stage timeout to finish rather than manually cancelling when possible: it leaves time for diagnostics upload. The workflow attempts upload on failure, but forced runner termination/cancellation can still prevent it. Heartbeats establish local liveness; no script can guarantee that GitHub browser live-log transport refreshes correctly. Consult archived logs and artifacts if the page stalls.
