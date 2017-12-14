# Vange-rs
[![Build Status](https://travis-ci.org/kvark/vange-rs.svg)](https://travis-ci.org/kvark/vange-rs)

[Vangers](https://www.gog.com/game/vangers) is a legendary game featuring unique gameplay and technical innovation.
The idea of this project is to replicate the old look and behavior, but with native hardware acceleration for the graphics.

You need the **original game** in order to try out `vange-rs`. The path to resources needs to be set in `config/settings.toml`.

### Instructions
```bash
git clone https://github.com/kvark/vange-rs
cd vange-rs
vi config/settings.toml # set the game path
cargo run --release
```

### Technonolgy

The game uses [gfx-rs](https://github.com/gfx-rs/gfx) for graphics and [glutin](https://github.com/tomaka/glutin) for context creation.

The level is drawn in a single full-screen draw call with a bit of ray tracing magic.

### Latest progress
![alt text](etc/shots/Road11-pause.png "WIP physics debugging on pause")
![alt text](etc/shots/Road7-vehicle.png "WIP screenshot of the world")
![alt text](etc/shots/Road10-debug-shape.png "WIP screenshot of the model")
