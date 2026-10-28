# OpenHE on Linux

## 1. Install the udev rule (required)

OpenHE talks to the keyboard through `hidraw`. Without a udev rule only root can open the device.
`70-openhe.rules` grants the logged-in user access with the `uaccess` tag:

```sh
sudo cp packaging/linux/70-openhe.rules /etc/udev/rules.d/70-openhe.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Then unplug and replug the keyboard. The rule matches vendor `1ca2`, products `1901` (WIN 68 HE MAX) and
`1902` (WIN 60 HE MAX). The file is numbered below 73 so that systemd's `73-seat-late.rules` turns the
`uaccess` tag into an ACL for the active session.

Check it worked: `getfacl /dev/hidraw*` should list `user:<you>:rw-` for the keyboard's hidraw node.
No group membership or `sudo` is needed afterwards.

An AppImage cannot install files into `/etc`, so this step is always manual. The rule is also bundled
inside the AppImage:

```sh
./OpenHE-*.AppImage --appimage-extract usr/share/openhe/70-openhe.rules
sudo cp squashfs-root/usr/share/openhe/70-openhe.rules /etc/udev/rules.d/
```

## 2. AppImage

Build prerequisites on Debian/Ubuntu (the same list CI installs):

```sh
sudo apt-get install -y --no-install-recommends \
  libudev-dev pkg-config libxkbcommon-dev libgl1-mesa-dev \
  libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
  libdbus-1-dev libxcb1-dev curl file
```

`libdbus-1-dev` and `libxcb1-dev` are for the foreground-app watcher behind per-app profile switching
(`active-win-pos-rs`: `libdbus-sys` is found through pkg-config `dbus-1`, and `xcb` links `libxcb`).
At run time they need only `libdbus-1-3` and `libxcb1`, which every desktop install has.

Build (needs the libraries above; `linuxdeploy` is downloaded
automatically unless it is on `PATH` or set with `LINUXDEPLOY=/path/to/linuxdeploy`):

```sh
packaging/linux/build-appimage.sh
```

The result is `dist/linux/OpenHE-<version>-x86_64.AppImage`. Run it with `chmod +x` and a double click, or
`./OpenHE-*.AppImage`. On systems without FUSE 2 use `./OpenHE-*.AppImage --appimage-extract-and-run`.

`packaging/linux/openhe.desktop` and the PNG icons from `assets/icons/png/` are embedded by the script.
The window class in the desktop file (`StartupWMClass=openhe-app`) must match the `app_id` the app sets
in eframe; adjust it if the app uses another one.

Notes:
- eframe loads Wayland, X11 and OpenGL libraries at run time (`dlopen`), so the AppImage uses the
  host's graphics stack. Build on an old enough distribution (Ubuntu 22.04) for the widest compatibility.
- Only x86_64 is built by CI. For aarch64 run the script on an aarch64 machine.

## 3. Debian package (optional, not built by CI)

`cargo-deb` can produce a `.deb` once the app crate has a `[package.metadata.deb]` section. A minimal
layout that installs everything the app needs:

| Source | Destination |
|---|---|
| `target/release/openhe-app` | `/usr/bin/openhe-app` |
| `packaging/linux/openhe.desktop` | `/usr/share/applications/openhe.desktop` |
| `assets/icons/png/openhe-256.png` | `/usr/share/icons/hicolor/256x256/apps/openhe.png` |
| `packaging/linux/70-openhe.rules` | `/lib/udev/rules.d/70-openhe.rules` |

With a `.deb` the udev rule is installed by the package, so step 1 is not needed. Add a `postinst`
that runs `udevadm control --reload-rules && udevadm trigger`.

## 4. Troubleshooting

- "No keyboard found": check `lsusb | grep 1ca2` first. If it is listed, the udev rule is missing or the
  keyboard was not replugged after installing it.
- Flatpak or Snap sandboxes block `hidraw` by default; use the AppImage or a native package.
