# Embedded power and heat

This records what a Radxa ZERO 3W running the Rune kiosk draws and why it runs hot, measured on 2026-11-17, and what `rune-kiosk.service` does about it.

## The board

Radxa ZERO 3W: RK3566, 8 GB of memory, Mali-G52 through panfrost, Rockchip 5.10 kernel under DietPi (Debian 12). The screen is a WaveShare 640 by 480 HDMI touchscreen that takes its power from the board's USB port. A brass heatsink sits on the SoC. The kiosk is the CI arm64 bundle on flutter-pi.

## How it was measured

- Power: a charger with a built-in meter that shows the current in steps of 0.1 A at 5 V, so every reading is good to about half a watt. Readings include the screen.
- Temperature: the SoC sensor, `/sys/class/thermal/thermal_zone0/temp`.
- Frames: `drm:drm_vblank_event_delivered` counted by perf.

The meter is too coarse to tell single controllers apart, and the temperature takes more than three minutes to settle with the heatsink on, so short temperature windows understate a change. Treat the wattages as steps of half a watt and the temperatures as a ranking.

## What was found

Rune is a small part of the heat. With the kiosk and the desktop both stopped the SoC sat at 62 to 63 °C; with the kiosk idle it sat at 63 °C.

The screen takes half of the power. The whole setup idles at 4 W. With the HDMI signal off it draws 2 W, so the screen, with its backlight and its HDMI bridge, accounts for about 2 W. That half turns into heat in the screen, away from the SoC.

The default CPU governor wastes power on animation. DietPi sets ondemand to jump to the top frequency above 50 % load and stay there for a second. The cover wall held the CPU at 1.8 GHz, which needs 1.1 V, while it keeps 60 frames a second at any limit down to 600 MHz. Every step up to 816 MHz runs at 0.85 V. No governor found the low step by itself: with no limit, schedutil averaged 1.57 GHz and conservative 1.79 GHz on the cover wall.

The board powers controllers it has no connector for. The ZERO 3W has no Ethernet port and nothing wired to its four USB 2 host controllers, and the system image binds drivers to all of them.

The heatsink takes heat in quickly and gives it up slowly. After a full load stopped, the SoC fell from 85 to 69 °C in ten seconds and then took eighty more to reach 63 °C. Four busy cores at 1.8 GHz reached the 85 °C throttling point in 25 seconds. A device built around this board needs a path from the SoC to a larger surface, such as a metal case.

Rune itself takes 177 MB of memory (proportional set size of the flutter-pi process) with the library on this board.

## Readings

Whole setup, screen included, at 5 V:

| State | Default | With the kiosk's settings |
|---|---|---|
| Idle | 0.8 A, 4.0 W | 0.7 A, 3.5 W |
| Cover wall, 60 frames a second in both | 0.9 A, 4.5 W | 0.8 A, 4.0 W |
| SoC after two minutes of cover wall | 77.5 °C, still rising | 69 to 70 °C |

Full load, for scale:

- Four busy cores at 1.8 GHz: 1.1 A, 5.5 W, throttled at 85 °C.
- Four busy cores limited to 1.1 GHz: 0.9 A, 4.5 W, 76 °C after 45 seconds.
- HDMI signal off, idle: 0.4 A, 2.0 W.

## What the kiosk does

While `rune-kiosk.service` runs, three helpers in `/opt/rune-embedded/bin` change the machine, and each puts back what it found when the kiosk stops. A restart or a crash keeps the remembered values in `/run`.

- `rune-gpu-devfreq` sets the GPU governor and its minimum frequency.
- `rune-cpu-cpufreq` limits every CPU to `RUNE_CPU_MAX_FREQ`. Work that fills the CPU, such as scanning or analysing a library, takes longer under the limit.
- `rune-unused-devices` unbinds the controllers in `RUNE_UNUSED_DEVICES` from their drivers. A controller with a network link up or a USB device plugged in stays on.

## Switching the display off

Of the 4 W the whole setup idles at, 2 W goes when the HDMI signal is off, which is the largest single saving found. The kiosk's flutter-pi switches the display off after a time without a touch.

- The app sets the time with the method call `setBlankAfter(seconds)` on the channel `rune/display_power` (`DisplayBlanker` in `lib/utils/display_blank_option.dart`), at start, from `RUNE_BLANK_AFTER` in `/etc/rune-kiosk.conf`, 60 by default. A settings page can call the same method later. 0 keeps the display on. Another embedder does not answer, and the display stays on.
- When the time is up flutter-pi sends `AppLifecycleState.paused`, so Flutter schedules no frames, and switches the display off through the connector's DPMS property a third of a second later. A display with no DPMS property is left on and Flutter is not paused.
- While the display is off, every touch is dropped. Two touches starting within half a second of each other switch it on, send `AppLifecycleState.resumed`, and are dropped too, along with the rest of that tap, so the double tap does not press whatever lies under it. A key press wakes the display as well. The touchscreen reports touches with the HDMI signal off: 388 events in 20 seconds of tapping.
- The patch is `linux/embedded/flutter-pi-display-blank.patch`, applied by `scripts/build_embedded.sh` after the partial repaint patch.

On the ZERO 3W with a 15 second timeout set for the test: the display went off 15 seconds after the last touch; for the next 20 seconds no frames were presented and the process used under 10 % of a core (against 60 frames a second and about 100 % with the cover wall on); a single tap did nothing; a double tap on the Tracks tile woke the display and left the home page as it was, and the next tap opened Tracks; the display went off again after the same time. The backend's own wake-ups stay: see below.

## Which machine gets which values

The numbers above were measured on one chip, and a limit that suits it can make another one slow. So the values are kept by chip, and the device tree says which chip a machine has.

- `/opt/rune-embedded/soc/<name>.conf` holds the values measured for a chip, where `<name>` is an entry of `/proc/device-tree/compatible`, such as `rockchip,rk3566`. The helpers read the profile of every entry the machine lists. A machine whose chip has no profile is left as it is.
- `/etc/rune-kiosk.conf` gives none of these settings as installed. A value given there replaces the chip's, and an empty value leaves that part of the machine alone.
- Which controllers have nothing wired to them depends on the board, which a chip profile cannot know, so no profile sets `RUNE_UNUSED_DEVICES`. The service unbound the ZERO 3W's Ethernet controller and four USB 2 hosts for the readings above; the overlay below now does that for the board.

To add a chip, measure it as described here and add its profile to `linux/embedded/soc`. The RK3566 profile has the GPU settings and the 816 MHz limit. The RK3568 profile has the GPU settings only, since the limit was not measured there.

After changing the service or a helper, check on a board that start, restart, a crash, stop and a crash loop still hand the screen over, and that the CPU limit and the GPU settings come back after a stop.

## A device tree overlay for the board

Unbinding a driver leaves some of a controller's clocks running: the Ethernet clock stayed enabled. `linux/embedded/boards/radxa-zero-3w-power.dts` disables the Ethernet controller, the four USB 2 hosts with their PHY, and the eMMC controller in the device tree, so they never start and their clocks stay gated. Its header says how to build and install it. With it the idle kiosk settled at 55.7 °C after nine minutes, against 57.2 °C with the controllers unbound by the service. The touchscreen and the kiosk work as before.

The overlay belongs to the board and its system image, so the bundle does not install it.

## Tried without effect

With the kiosk idle under its settings the SoC settles at 57 °C after five minutes, against 62 to 63 °C before them. On top of that, each of these was held for five minutes, and together they lowered the SoC by half a degree, which is within the noise:

- Wi-Fi power saving on (`iwconfig wlan0 power on`). The module is a separate chip, so a saving there may not reach the SoC sensor; the meter showed no step either.
- The eMMC controller unbound (`sdhci-dwcmshc/fe310000.sdhci`). This board has no eMMC and boots from the SD card.
- The USB OTG controller unbound (`dwc3/fcc00000.dwc3`).

They are left out of the overlay.

## Still open

- Most of the board's own draw, about 1.5 W, is unexplained.
- The USB 3 PHY stays powered though the touchscreen uses USB 2 signals only. An overlay that disabled the PHY (`combphy1_usq`) and gave the host controller the USB 2 PHY alone with `maximum-speed = "high-speed"` brought the controller up, and the touchscreen was no longer found on it. It was taken out again.
- The screen's backlight is the largest single consumer. Dimming it and turning it off with the screen need a panel with a backlight input.
- Handhelds built on the same RK3566 advertise about six hours on a 12 to 13 Wh battery, which is about 2 W with the screen on. They use a DSI panel and a device tree written for the device. These figures come from memory of their specifications and were not checked; if they hold, that is the mark for a player built on this SoC.
- The kiosk is never fully idle. Paused on the library home page the process uses about 15 % of a core. A 20 second profile puts about a third of that in the backend: the loops in `native/hub/src/server/sync_worker.rs` (`verify_managed_audio`, `run_share_worker`, `run_loop`, `maintain_metadata`, `schedule_peers`) wake every second and run SQLite transactions. Their threads carry the name `io.flutter.ui`, inherited from the thread that started them, so per-thread figures count them as the UI thread. The home page also flips its cover tiles every eight seconds, which draws about 32 frames each time.
- Nothing suspends the board when playback is paused for a long time.
- With the display off the backend's sync loops are the remaining wake-ups. They could check only while the display is on, with a sync started by hand running to the end either way.
