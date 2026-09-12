# Multiplayer (native + web)

Story in vange-rs is the world **cycle** (cirt gather → escave delivery →
palette / light change), implemented in `src/level/cycle.rs`. In multiplayer
the **server** owns gather / deliver / bank decay / stage advance and
broadcasts a `CycleState` on every `WorldState`. Clients keep a local `Bunch`
for palette fades and apply the snapshot so native TCP and web WS stay aligned.

## Run

Terminal A — authoritative server. With `config/settings.ron` and game data,
omitting `--level` loads the same world as clients (`settings.game.level`, e.g.
Fostral → `thechain/fostral/world.ini`). Use `--level test` for the procedural
CI / smoke level (no story cycle). Welcome / HUD `level_name` always matches
the terrain the server actually hosts.

```bash
# Shared-world playtest (default when data + settings.game.level are present):
cargo run -p vangers-server -- --port 7800 --ws-port 7801

# Explicit Fostral (world name or world.ini path):
cargo run -p vangers-server -- --port 7800 --ws-port 7801 --level Fostral
# cargo run -p vangers-server -- --port 7800 --ws-port 7801 --level /path/to/thechain/fostral/world.ini

# Procedural test level (CI / integration tests):
cargo run -p vangers-server -- --port 7800 --ws-port 7801 --level test
```

Terminal B — native client:

```bash
cargo run --bin road -- --server 127.0.0.1:7800 --name Alice
```

Terminal C — another native client, or web:

```bash
# Web (compile-time WebSocket URL; rebuild when the host changes):
VANGERS_SERVER_WS=ws://127.0.0.1:7801 cargo build --target wasm32-unknown-unknown --features web --bin web
# then serve the wasm/js/html as usual for the web target
```

Protocol: TCP `7800` (native), WebSocket `7801` (wasm). Messages live in
`lib/net` (`ClientMessage` / `ServerMessage`). `WorldState` carries
`agents` plus optional `cycle: Option<CycleState>` (stage index, banks,
light, fade progress, per-player cirtainer).

## Tests

```bash
cargo test -p vangers-net
cargo test --test net_physics
# cycle unit tests (incl. sync_authority):
cargo test --lib level::cycle
```

See the header comment in `tests/net_physics.rs` for how to extend that
integration test with `CycleState` round-trips.
