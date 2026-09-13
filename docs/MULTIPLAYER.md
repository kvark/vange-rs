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
`agents` (pose, dynamo, **`spiral_charge`**) plus optional
`cycle: Option<CycleState>` (stage index, banks, light, fade progress,
per-player cirtainer).

## Player identity on reconnect

The server reclaims `player_id` by **Join name** (`--name` / UI name). After leave
or disconnect, joining again with the same name receives the same `player_id` in
`Welcome` (and the same agent slot in `WorldState` / cycle carry). Different
names still get distinct ids. No protocol change: clients keep sending
`ClientMessage::Join { player_name, ... }` as today.

For a short **grace window** (currently 5 minutes), the server also keeps that
player's last `WorldState` transform / dynamo, per-player cycle carry
(`Cirtainer` / `CycleState.players[].held`), **and spiral charge**. A same-name
reclaim inside the window restores pose, cirtainer, and spiral together instead
of a fresh spawn, so leave→rejoin keeps story progress continuous on a shared
world. World-level banks / stage / fade stay on the server `Bunch` for everyone
either way. After the grace expires (or for a brand-new name), spawn and carry
are fresh as before. Other players are unaffected.

```bash
# Terminal B — gather some cirt, leave (Ctrl+C), rerun with the same --name:
cargo run --bin road -- --server 127.0.0.1:7800 --name Alice
# Welcome player_id matches; you reappear near the prior XY with the same held cirt.
```


## Spiral charge (WorldState + reclaim)

Spiral energy (passage fuel) is **server-tracked per player** as
`AgentState.spiral_charge`. Capacity stays client-local (car `max_teleport`).
Solo offline still charges/discharges only in memory; multiplayer clients
report fills and spends with `ClientMessage::SetSpiral { charge }` after
escave / outdoor `KEY_UPDATE` charge or a successful passage hop. The server
echoes the value on every `WorldState`, so Alice’s HUD and Bob’s Multiplayer
panel / log stay aligned, and a same-name reclaim inside the grace window
restores the charged spiral with pose and cirtainer.

```bash
# Terminal A — server (matching level)
cargo run -p vangers-server -- --port 7800 --ws-port 7801

# Terminal B / C — Alice charges at a spiral station or escave; Bob watches
# Multiplayer panel / logs for Alice’s spiral_charge. Alice Ctrl+C and reconnects
# with the same --name → spiral still charged.
cargo run --bin road -- --server 127.0.0.1:7800 --name Alice
cargo run --bin road -- --server 127.0.0.1:7800 --name Bob
```

## Local player pose (blend vs snap)

Client and server both run physics. A hard snap on every 20 Hz `WorldState`
fights local contact — a wedged car gets slammed back into geometry (jump-mass
spam). The local player now blends small position/rotation errors toward the
server and hard-snaps only when `|Δpos|` (or rotation) exceeds a threshold
sized for 20 Hz authority (`Transform::WORLD_STATE_SNAP_POS` / `_SNAP_ROT` /
`_BLEND`). The first snapshot still hard-snaps camera and pose. Tweaks
Position hold / `SetPose` is unchanged.

## Tweaks Position (debug hold + SetPose)

In multiplayer, `WorldState` corrects the local player's transform toward the
server each tick (soft blend under a pose-error threshold; hard-snap above it).
Editing **Player → Position** in the Tweaks panel (egui
`DragValue`) would otherwise fight that snap. While those X/Y controls are
focused or being dragged — and for a short hold after — the client skips
applying the server transform/dynamo to the local agent only.

When the edit session ends (controls lose focus / drag stops), or when the hold
timer expires with a pending edit, the client sends `ClientMessage::SetPose`
with the local position and orientation. The server applies that transform for
the sender's `player_id` (and clears residual velocity) so the next `WorldState`
broadcast keeps the tweaked spot for everyone. Remote agents and normal sync
when not editing are unchanged. **Debug-only** — not for normal gameplay.

```bash
# Terminal A — server (matching level as usual)
cargo run -p vangers-server -- --port 7800 --ws-port 7801

# Terminal B / C — Alice and Bob
cargo run --bin road -- --server 127.0.0.1:7800 --name Alice
cargo run --bin road -- --server 127.0.0.1:7800 --name Bob
# In Alice Tweaks → Player → Position: drag X/Y; release / unfocus.
# Alice stays at the edited spot (no snap-back). Bob sees Alice there.
```

## Tests

```bash
cargo test -p vangers-net
cargo test --test net_physics
# local WorldState blend vs snap:
cargo test --lib space::reconcile_tests
# cycle unit tests (incl. sync_authority):
cargo test --lib level::cycle
# server integration (incl. SetPose / SetSpiral → WorldState, reclaim):
cargo test -p vangers-server --test integration
```

See the header comment in `tests/net_physics.rs` for how to extend that
integration test with `CycleState` round-trips.
