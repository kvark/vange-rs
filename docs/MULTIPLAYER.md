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

## Player identity on reconnect

The server reclaims `player_id` by **Join name** (`--name` / UI name). After leave
or disconnect, joining again with the same name receives the same `player_id` in
`Welcome` (and the same agent slot in `WorldState` / cycle carry). Different
names still get distinct ids. No protocol change: clients keep sending
`ClientMessage::Join { player_name, ... }` as today.

For a short **grace window** (currently 5 minutes), the server also keeps that
player's last `WorldState` transform / dynamo **and** per-player cycle carry
(`Cirtainer` / `CycleState.players[].held`). A same-name reclaim inside the
window restores pose and cirtainer together instead of a fresh spawn, so
leave→rejoin keeps story progress continuous on a shared world. World-level
banks / stage / fade stay on the server `Bunch` for everyone either way.
After the grace expires (or for a brand-new name), spawn and carry are fresh
as before. Other players are unaffected.

```bash
# Terminal B — gather some cirt, leave (Ctrl+C), rerun with the same --name:
cargo run --bin road -- --server 127.0.0.1:7800 --name Alice
# Welcome player_id matches; you reappear near the prior XY with the same held cirt.
```


## Tweaks Position (debug hold)

In multiplayer, `WorldState` normally snaps the local player's transform to the
server each tick. Editing **Player → Position** in the Tweaks panel (egui
`DragValue`) would otherwise fight that snap. While those X/Y controls are
focused or being dragged — and for a short hold after — the client skips
applying the server transform/dynamo to the local agent only. Remote agents and
normal sync when not editing are unchanged. This is a debug-only local hold, not
an authoritative teleport; after the hold expires the next `WorldState` snaps
again.

```bash
# Terminal A — server (matching level as usual)
cargo run -p vangers-server -- --port 7800 --ws-port 7801

# Terminal B / C — Alice and Bob
cargo run --bin road -- --server 127.0.0.1:7800 --name Alice
cargo run --bin road -- --server 127.0.0.1:7800 --name Bob
# In Alice Tweaks → Player → Position: drag X/Y; value sticks for the edit
# session. Bob still sees Alice move when she is not tweaking.
```

## Tests

```bash
cargo test -p vangers-net
cargo test --test net_physics
# cycle unit tests (incl. sync_authority):
cargo test --lib level::cycle
```

See the header comment in `tests/net_physics.rs` for how to extend that
integration test with `CycleState` round-trips.
