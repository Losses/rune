# Embedded power and heat

This records what a Radxa ZERO 3W running the Rune kiosk draws and why it runs hot, measured on 2026-11-17, and what `rune-kiosk.service` does about it. The settings are in `linux/embedded/rune-kiosk.conf`.

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
- `rune-cpu-cpufreq` limits every CPU to `RUNE_CPU_MAX_FREQ`, 816 MHz by default. Work that fills the CPU, such as scanning or analysing a library, takes longer under the limit.
- `rune-unused-devices` unbinds the controllers in `RUNE_UNUSED_DEVICES` from their drivers: by default the Ethernet controller and the four USB 2 hosts of the ZERO 3W. A controller with a network link up or a USB device plugged in stays on.

Each setting can be emptied in `/etc/rune-kiosk.conf` to leave that part of the machine alone. The installer keeps an existing `/etc/rune-kiosk.conf`, so a board installed before these settings existed needs them copied in from the bundle's `rune-kiosk.conf`.

After changing the service or a helper, check on a board that start, restart, a crash, stop and a crash loop still hand the screen over, and that the CPU limit and the controllers come back after a stop.

## Still open

- About 1.5 W of the board's own draw is unexplained. Still powered with nothing using them: the USB 3 PHY and the OTG controller, the eMMC controller, and the Wi-Fi module with power saving off. The Ethernet clock stays enabled after its driver is unbound, so a device tree that disables these outright may save more than unbinding does.
- The screen's backlight is the largest single consumer. Dimming it and turning it off with the screen need a panel with a backlight input.
- Handhelds built on the same RK3566 advertise about six hours on a 12 to 13 Wh battery, which is about 2 W with the screen on. They use a DSI panel and a device tree written for the device. These figures come from memory of their specifications and were not checked; if they hold, that is the mark for a player built on this SoC.
- With the picture still, the kiosk's UI thread uses about 6 % of a core and the process about 11 %.
- Nothing suspends the board when playback is paused for a long time.
