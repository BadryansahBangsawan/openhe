# OpenHE

A lightweight, native configurator for AULA WIN 68 HE MAX and WIN 60 HE MAX
Hall-effect keyboards, written in Rust. It runs on Windows, Linux and macOS,
talks to the keyboard directly over USB, and needs no browser, account or
internet connection.

> Status: design stage. No app code yet.

## Planned features

- Per-key actuation point, rapid trigger (press/release sensitivity), top and bottom deadzones
- Advanced keys: DKS, MT, TGL, MPT, SOCD, END
- Remapping on 4 layers, macros
- Lighting: effects, per-key RGB, brightness, speed
- Polling rate (125 Hz – 8 kHz), 4 onboard profiles, Win/Mac mode
- Calibration, factory reset, live key-depth view

Firmware updates are out of scope.

## Docs

- [`docs/protocol/PROTOCOL.md`](docs/protocol/PROTOCOL.md): HID protocol notes, from a static
  read of the official AULA HUB 1.1.22 app
- [`assets/logo/concepts/`](assets/logo/concepts/): logo concepts

## Supported devices

| Model | VID | PID | Config interface |
|---|---|---|---|
| WIN 68 HE MAX | `0x1CA2` | `0x1901` | usage page `0xFFA0`, 64-byte reports |
| WIN 60 HE MAX | `0x1CA2` | `0x1902` | usage page `0xFFA0`, 64-byte reports |

OpenHE is not affiliated with AULA.
