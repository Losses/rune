#!/usr/bin/env bash
# Run on a macOS desktop from the repository root. No system settings change.
set -euo pipefail
if [[ "$(uname -s)" != Darwin ]]; then echo 'Run this script on macOS.' >&2; exit 1; fi
printf 'macOS: '; sw_vers -productVersion
printf 'Architecture: '; uname -m
flutter --version
system_profiler SPDisplaysDataType
printf '%s\n' 'Record each display scaling choice in System Settings > Displays.'
printf '%s\n' 'Follow documents/application-scaling.md#native-acceptance-cases for physical IME, drag/drop, traffic-light and monitor-transition checks.'
if [[ "${1:-}" == --widget-tests ]]; then
  flutter test --no-pub test/providers/app_viewport_test.dart test/widgets/scaled_flyout_test.dart test/widgets/app_drop_target_test.dart test/widgets/fluent_popup_metrics_test.dart
  printf '%s\n' 'These widget tests do not validate native macOS input or displays.'
fi
