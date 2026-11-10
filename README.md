# Soundhold

Head tracked spatial sound for Bluetooth headphones on Linux with PipeWire. Sound stays in place in the room as you turn your head.

Not affiliated with or endorsed by Sony.

## Requirements

- PipeWire with the filter-chain module and the SOFA plugin. On Ubuntu: `libpipewire-0.3-modules` and `libspa-0.2-modules-extra`.
- BlueZ and WirePlumber.
- Headphones that provide the Android head tracker Bluetooth service.

## Building

On Ubuntu:

```
sudo apt install libpipewire-0.3-dev libclang-dev libgtk-4-dev libadwaita-1-dev
cargo build
```

`libgtk-4-dev` and `libadwaita-1-dev` are only needed for the window. Without them, build with `cargo build -p soundhold`.

To build `.deb` packages in `target/deb`, install `dpkg-dev` and run `sh packaging/build-deb.sh`.

## Access to the head tracker

Install the access rule once:

```
sudo sh install-rule.sh
```

The rule covers Sony Bluetooth devices only.

## Usage

The window starts the background program, which shows a status icon in the top bar and keeps running when the window closes.

```
cargo build
target/debug/soundhold-window
```

| Command | Action |
|---|---|
| `soundhold daemon` | Run the background program |
| `soundhold stop` | Stop the background program |
| `soundhold autostart on` or `off` | Start the background program at login |

While spatial sound is on, the microphone does not switch on automatically. It does when spatial sound is off.

To run in a terminal without the background program:

```
cargo run -p soundhold -- run
```

| Key | Action |
|---|---|
| `c` | Set the current direction as centre |
| `b` | Switch between spatial and normal sound |
| `q` | Quit |

`--codec NAME` selects the Bluetooth codec, for example `cargo run -p soundhold -- run --codec sbc_xq`. An unknown name lists the codecs the headphones offer.

The window saves sound width, level, codec and head data in `~/.config/soundhold/settings.json`. Head data files added in the window are copied to `~/.local/share/soundhold/heads`.

## Limitations

- Only tested on the Sony WF-1000XM5 on Ubuntu 26.04 with PipeWire 1.6.2.

## Testing other headphones

```
cargo run -p soundhold -- check
```

Please open an issue with the output and your headphone model. To report which settings work best, use Share setup in the window.

## Credits

- Head measurements: `sofa/HRIR_FULL2DEG.sofa` and `sofa/fhk_circ360.sofa`, Neumann KU100 HRIR, Benjamin Bernschütz, [Technische Hochschule Köln](https://audiogroup.web.th-koeln.de/ku100hrir.html), [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/). Files are unmodified.
- [SonyTrackerLinux](https://github.com/kdani3/SonyTrackerLinux) (MIT) and [sony-head-tracker](https://github.com/NicholasSlattery/sony-head-tracker) documented how to read the sensor.

## License

The code is licensed under GPL-3.0. See `LICENSE`.

The files in `sofa/` are licensed under CC BY-SA 3.0.
