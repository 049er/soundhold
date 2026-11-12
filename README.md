# Soundhold

Head tracked spatial sound for Bluetooth headphones on Linux. Sound stays in place in the room as you turn your head.

Soundhold reads the head sensor in the headphones and plays the sound from the computer as if it came from speakers in front of you. It needs PipeWire and headphones with a head sensor that follows the Android head tracker format. It is only tested on the Sony WF-1000XM5.

## Install

Download the packages from the [latest release](https://github.com/049er/soundhold/releases/latest). `soundhold` is the background program and the command. `soundhold-window` is the window. The packages are for x86_64 and are built on Ubuntu 26.04 and Fedora 45.

To check the downloaded files, run this in the folder that holds them and `SHA256SUMS`:

```
sha256sum -c --ignore-missing SHA256SUMS
```

### Ubuntu

```
sudo apt install ./soundhold_*.deb ./soundhold-window_*.deb
```

### Fedora

```
sudo dnf install ./soundhold-*.rpm
```

### Arch Linux

The `PKGBUILD` in `packaging/aur` builds both packages from the latest release tag:

```
cd packaging/aur
makepkg -si
```

### Remove

On Ubuntu:

```
sudo apt remove soundhold soundhold-window
```

On Fedora:

```
sudo dnf remove soundhold soundhold-window
```

## Usage

Connect the headphones and open Soundhold from the app menu, or run `soundhold-window`.

The window starts the background program. The background program shows a status icon in the top bar and keeps running when the window closes. On GNOME the icon needs the AppIndicator extension, which Ubuntu includes.

The window has these controls:

| Control | Action |
|---|---|
| Switch in the status card | Switch between spatial and normal sound |
| Set centre | Set the current direction as centre |
| Sound width | Angle between the centre and each side speaker |
| Spatial level | Volume of the spatial sound |
| Head data | The head measurement file (`.sofa`) used for the sound |
| Bluetooth codec | The Bluetooth sound format the headphones use |
| Start at login | Start the background program at login |
| Enable app | Start or stop the background program |

The background program saves sound width, level, codec and head data in `~/.config/soundhold/settings.json`. Head data files added in the window are copied to `~/.local/share/soundhold/heads`.

While spatial sound is on, the microphone of the headphones does not switch on automatically for calls. When spatial sound is off, this follows your WirePlumber setting again.

### Commands

| Command | Action |
|---|---|
| `soundhold daemon` | Run the background program |
| `soundhold stop` | Stop the background program |
| `soundhold autostart on` or `off` | Start the background program at login, or not |
| `soundhold check` | Test the head sensor of the connected headphones |
| `soundhold run` | Run in a terminal without the background program |

`soundhold run` does not start while the background program runs, and it does not use the saved settings. It has these keys:

| Key | Action |
|---|---|
| `c` | Set the current direction as centre |
| `b` | Switch between spatial and normal sound |
| `q` | Quit |

`--codec NAME` selects the Bluetooth codec, for example `soundhold run --codec sbc_xq`. An unknown name lists the codecs the headphones offer.

## Tested hardware

- Only tested on the Sony WF-1000XM5 on Ubuntu 26.04 with PipeWire 1.6.2.

Not affiliated with or endorsed by Sony.

## Testing other headphones

Connect the headphones and run:

```
soundhold check
```

Please open an issue with the output and your headphone model. The window shows a Report these headphones button when it finds no working head sensor, which fills in the issue for you. To report which settings work best, use Share setup in the window.

## Access to the head sensor

Linux gives only root access to the raw device files in `/dev/hidraw*` that carry the head sensor data. The packages install an access rule for it. When building from source, install the rule once:

```
sudo sh install-rule.sh
```

The rule in `70-soundhold.rules` matches Bluetooth devices whose report description starts with the HID custom sensor usage, which the Android head tracker format uses. Keyboards and mice do not start with this usage.

It gives the user signed in at the computer read and write access to that device file, the same access desktop sessions get for sound cards and game controllers. To check a Bluetooth device, the rule runs `/bin/sh`, `od` and `grep` as root to read the first four bytes of its report description. It grants nothing else.

To remove the rule after `install-rule.sh`:

```
sudo rm /etc/udev/rules.d/70-soundhold.rules
sudo udevadm control --reload
```

Removing the package removes its copy of the rule.

## Building from source

The packages install what Soundhold needs to run. A build from source needs these installed:

- PipeWire with its command line tools, the filter-chain module and the SOFA plugin. On Ubuntu: `pipewire-bin`, `libpipewire-0.3-modules` and `libspa-0.2-modules-extra`.
- BlueZ and WirePlumber.

On Ubuntu, with Rust installed:

```
sudo apt install build-essential pkg-config libclang-dev libpipewire-0.3-dev libgtk-4-dev libadwaita-1-dev
cargo build
target/debug/soundhold-window
```

`libgtk-4-dev` and `libadwaita-1-dev` are only needed for the window. Without them, build with `cargo build -p soundhold` and run `target/debug/soundhold`.

Use the debug build to run from the source folder. A release build looks for the head data in `/usr/share/soundhold`, where the packages install it.

### Packages

To build `.deb` packages in `target/deb`, install `dpkg-dev` and `git` and run `sh packaging/build-deb.sh`. The version gets a build time and commit suffix, in the form `VERSION+gitDATETIME.COMMIT`, so each build installs over the previous one.

To build `.rpm` packages in `target/rpm/RPMS` on Fedora, install `rpm-build` and run `sh packaging/build-rpm.sh`. The build needs `clang-devel`, `pipewire-devel`, `gtk4-devel` and `libadwaita-devel`.

Pushing a `v*` tag builds the packages and creates a GitHub release.

## Credits

- Head measurements: `sofa/HRIR_FULL2DEG.sofa` and `sofa/fhk_circ360.sofa`, Neumann KU100 HRIR, Benjamin Bernschütz, [Technische Hochschule Köln](https://audiogroup.web.th-koeln.de/ku100hrir.html), [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/). Files are unmodified.
- [SonyTrackerLinux](https://github.com/kdani3/SonyTrackerLinux) (MIT) and [sony-head-tracker](https://github.com/NicholasSlattery/sony-head-tracker) documented how to read the sensor.

## License

The code is licensed under GPL-3.0-only. See `LICENSE`.

The files in `sofa/` are licensed under CC BY-SA 3.0.
