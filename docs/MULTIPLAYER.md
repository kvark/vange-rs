# Multiplayer (native + web)

Story in vange-rs is the world **cycle** (cirt gather → escave delivery →
palette / light change), implemented in `src/level/cycle.rs`, plus **moving
land** (`*.vot` patches in `src/level/moving.rs`). In multiplayer the
**server** owns gather / deliver / bank decay / stage advance and moving-land
playback, and broadcasts `CycleState` / `MovingLandState` on every
`WorldState`. Clients apply those snapshots so native TCP and web WS stay
aligned (solo still advances locally).

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
per-player cirtainer) and optional `moving: Option<MovingLandState>`
(per-location frame / stage / go_phase / step).

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


## Moving land (WorldState phase sync)

Animated terrain patches (`*.vot` / `MovingWorld`) are **server-owned** in
multiplayer. Each tick the server runs one moving-land quant (sensors see all
joined players) and broadcasts `MovingLandState` on `WorldState`. Connected
clients **apply** that snapshot instead of calling `MovingWorld::step` locally,
so Tweaks “Moving land frame/phase” matches across Alice/Bob and joiners catch
up immediately. Solo / offline still free-runs locally as before.

```bash
# Terminal A — server (world with moving land, e.g. Fostral)
cargo run -p vangers-server -- --port 7800 --ws-port 7801

# Terminal B / C — compare Tweaks → Moving land frame/phase
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

Both sides step the car in whole `max_quant` quanta (`physics::QuantClock`,
the remainder carried to the next tick or frame), apply controls per
quantum, and release a jump on the last quantum of the server tick / client
frame. `physics::step` is not linear in `dt`, so the old "quanta plus a
remainder step per call" split flew the same jump along a different path at
every client frame rate (9–90u apart on landing on Fostral). Only the newest
`WorldState` of a poll batch is reconciled against the local car; older ones
in the same slow frame would pull it back along its own path.

## Duel spawn (ram / bump tests)

`--duel-spawn` places the first two **fresh** joiners 70u apart (centre to
centre; a ~30u gap between the bumpers of ~40u-long mechos), facing each
other along X, so collision / bump tests need no long drive. Joiner 0 spawns
35u west of `--duel-center` facing east (+X); joiner 1 spawns 35u east facing
west (−X). Later joiners use the regular spawn grid, and a reconnect within the
grace window still restores its last pose. Without the flag spawning is
unchanged.

The default center `590,4151` is open flat sand on Fostral: terrain height 0–2
with no slab (dual) texels across a 50×16 box, and both cars settle upright in
place within about 1u. For other worlds pass a flat spot via
`--duel-center X,Y`.

```bash
# Terminal A — server
cargo run -p vangers-server -- --port 7800 --ws-port 7801 --level Fostral --duel-spawn
# cargo run -p vangers-server -- ... --duel-spawn --duel-center 590,4151

# Terminal B / C — join in order: Alice (west, faces east), then Bob (east, faces west)
cargo run --bin road -- --server 127.0.0.1:7800 --name Alice
cargo run --bin road -- --server 127.0.0.1:7800 --name Bob
```

## Car-vs-car contact (server-authoritative)

After every physics quantum of every joined player, the server runs
`physics::collide_cars` over every pair of players (in player-id order). It is
a port of the original `Object::test_object_to_baseobject`: a bounding-sphere
precheck, an oriented-box test on the `scale_box`-shrunk model bounds, the
original `box_to_box_intersection` contact point, and the original's fully
elastic impulse `P = -2 K⁻¹ u0` along the line between the centres, using both
cars' masses and inertia. The port also pushes overlapping cars apart by half
the overlap (beyond 0.1u) so traction cannot press one car into another. The
new velocities and poses reach both clients through `WorldState`. Bumps at or
above `strong_double_collision_threshould` from `common.prm` log
`Car contact tick … impulse=…` at info; lighter bumps and resting contact
log at debug. A slow ram is a run of light bumps: the rammer stops, the
parked car takes its speed, the rammer's traction catches up again, so the
pair moves on together about 44u apart (centre to centre) rather than
bouncing.

To trace both cars every tick while they are within 60u, run the server with
`RUST_LOG=info,vangers_server::contact=debug`:

```
Car proximity tick 178 dist=42.6 p1 pos=(582.8,4151.7,4.0) vel=(-0.03,0.23,-0.02) p2 pos=(625.4,4150.2,6.0) vel=(6.00,-0.22,0.21) contact=true depth=2.15 impulse=2.699
```

`vel` is world-frame; `depth`/`impulse` are 0 when the boxes do not touch.

The native client predicts the same contact for its own car against each
remote ghost (its latest `WorldState` pose and velocities), after each of its
own quanta like the server, so
the rammer bounces and the rammed car moves right away rather than passing
through the ghost until reconcile pulls it back. The ghost itself is never
moved locally. The box test is fully 3D, so airborne cars and cars on
different heights touch correctly; predicting once per render frame against
the server's once per tick is what used to leave the local car 10–20u off
after a bump.

Tuning: `RESTITUTION` (1.0 = original) and `SEPARATION` in
`src/physics/collision.rs`.

### Felt bumps (receiving side)

Local prediction alone under-delivers a hit on the rammed car: the rammer's
ghost arrives a tick or more late, already slowed by the server's contact,
so the predicted bump is a fraction of the real one (1.35 against 5.19 for
the duel-spawn ram), and the blend-only reconcile never copies velocity.
So the server counts, per car, the contacts whose velocity change
(`impulse / mass`, `CarContact::delta_v`) reaches `physics::FELT_BUMP_DV`
(1.0: a ram is ~5, resting contact ~0.1) and sends the wrapping counter as
`AgentState::bumps`. When the counter differs from the one seen at the
client's previous reconcile (robust to snapshots skipped in a slow frame),
`physics::reconcile_local_car` adopts the server's linear and angular
velocity, rotated into the client's own blended frame, while the pose keeps
its normal 0.25 blend: no new snap, and with matching velocities the offset
decays. The native client logs it at info:

```
Server bump: adopted vel=(4.92,-0.31,0.08) bumps=3
```

The web client uses the same reconcile; it still reconciles every snapshot
and steps once per frame (follow-up).

## Jump trace (server)

The server logs every jump at info, with the pose before the tick and the
world velocity after it, then the pose every second; the trace ends at the
first sample from +2s on where the car is at rest (speed < 0.2), or at +10s:

```
Jump tick 4210 p1 power=10.0 pos=(884.5,3960.3,47.2) vel=(…)
Jump trace +1.0s tick 4230 p1 pos=(…) vel=(…) moved=(…)
Jump trace +2.0s tick 4250 p1 pos=(…) vel=(…) moved=(…) at rest
```

`moved` is relative to the jump pose. A car that keeps sliding after
landing on a steep slope shows here: e.g. Fostral (884.5, 3960.3) sits on a
~50° slope (height 47 down to 3 within 37u along the fall line), and a
standing jump there slides the car down to the floor near (903, 3990) on the
server too.

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
# moving-land sync_authority:
cargo test --lib level::moving::tests::sync_authority_matches_free_running_peer
# server integration (incl. SetPose / SetSpiral → WorldState, reclaim, duel spawn):
cargo test -p vangers-server --test integration
```

See the header comment in `tests/net_physics.rs` for how to extend that
integration test with `CycleState` / `MovingLandState` round-trips.
