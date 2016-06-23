[Vangers](https://www.gog.com/game/vangers) is a legendary game featuring unique gameplay and technical innovation.
The idea of this project is to replicate the old look and behavior, but with native hardware acceleration for the graphics.

### Instructions
```bash
git clone https://github.com/kvark/vange-rs
git clone https://github.com/gfx-rs/gfx
cd vange-rs
mkdir .cargo && echo "paths = [\"../gfx\"]" > .cargo/config
nano config/settings.toml # set the game path
cargo run --release
```

### Latest progress
![alt text](etc/shots/Road5-color.png "WIP screenshot")
