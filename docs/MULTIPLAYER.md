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
# Web: the same bundle CI deploys (wasm-bindgen 0.2.117 into docs/), with
# the server's WebSocket compiled in.
VANGERS_SERVER_WS=ws://127.0.0.1:7801 cargo build --target wasm32-unknown-unknown --features web --bin web --release
wasm-bindgen target/wasm32-unknown-unknown/release/web.wasm --out-dir docs/ --target web --no-typescript
# Game data next to the page (the data-0 release: common.zip + <level>.zip).
mkdir -p docs/data-0 && cp /path/to/data-0/{common,fostral}.zip docs/data-0/
python3 -m http.server -d docs 8080
# open http://127.0.0.1:8080/#name=Bob&color=red
```

The web client takes its identity and server from the URL fragment:
`#name=Bob` (default `WebPlayer`; the server reclaims a player by name),
`#color=red` (`green`, `red`, `blue`, `yellow`, `gray`), and
`#server=ws://host:7801` to join a server the bundle was not built for (a
`VANGERS_SERVER_WS`-less build plays solo otherwise). It joins with the
first main car of `car.prm`, the same one a native client picks by default.
A build with a server keeps the full terrain height (`0x100`, what the
server steps the cars on); a solo build flattens it to `0x80` for the
original look. `#level=khox` picks the page's level (the level picker
does the same); it has to be the server's level, or the page logs
`Server runs 'X' but this page loaded 'Y'` and the status line says
`level mismatch`.

### Web without a GPU (software WebGL)

On a machine without a GPU, Chrome runs the web build on SwiftShader
(WebGL2): `google-chrome --use-angle=swiftshader --enable-unsafe-swiftshader`.
SwiftShader caps textures at 8192 px, so the page swaps every big level
(Fostral, Glorx, ... are 2048 x 16384) for Khox, the one stock level that
fits. A mixed session there needs a Khox server and Khox for the native
client. The `data-0` release has `khox.zip` (`gh release download data-0
-R kvark/vange-rs -p khox.zip -D docs/data-0`); unzip it into
`<data_path>/thechain/khox/` for the server and the native client (it holds
`world.ini`, which `wrlds.dat` lists as `Khox`). Then:

```bash
cargo run --release -p vangers-server -- --port 7810 --ws-port 7811 --level Khox \
    --duel-spawn --duel-center 410,195   # flat ground, first two joiners face each other
cargo run --release --bin road -- --server 127.0.0.1:7810 --name Alice
# open http://127.0.0.1:8080/#name=Bob&color=red&level=khox&server=ws://127.0.0.1:7811
```

Software rendering runs at about one frame a second or slower. In
multiplayer each frame still steps the quanta of the real time it took (up
to 2.5 s), so the web car keeps the server's pace; the server waits up to
3 s (`InputQueue::STARVE_TICKS`) for a quiet client's next batch before it
drives the car on with the held control. Those driven-on quanta are motion
the client never predicted: the server logs `Input pN (name) starved: ...`
and `... resumed after K unacked held quanta` when that happens. Solo, a
frame steps at most 0.1 s, so the car drives in slow motion. Under heavy
load,
`python3 -m http.server` (a listen backlog of 5) can drop one of the zip
fetches. The page then logs `Couldn't fetch ...` and falls back. Reload
the page, or use any other static server.

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

This is how both clients reconciled before client prediction (next
section); `physics::reconcile_local_car` is kept for the comparison in
`tests/lag_compensation.rs`. Client and server both run physics. A hard snap on every 20 Hz `WorldState`
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

## Lag compensation (client prediction, protocol 2)

Both clients predict their own car and let the server's word land
without visible snaps. The building blocks are in
`src/physics/prediction.rs`; `src/physics/client.rs` puts them together
for a client (`ClientSync`: quantum clock, numbered history, ghosts,
render offset; `RemoteCar`: a remote player's snapshots, interpolated
pose and ghost), and the native game (`bin/road/game.rs`) and the web
build (`bin/web/main.rs`) both run it.

- **Numbered quantum inputs.** Each frame the client steps its car in whole
  `max_quant` quanta and sends one `ClientMessage::Input { first_quant,
  quanta, control }`; the held controls cover every quantum, jump / roll the
  last one. The server queues them per player (`InputQueue`) and runs
  exactly those quanta, one tick's worth per tick. It does not race through
  a burst (a long client frame): other cars run at the tick rate, and a
  client bumping into them predicts them on its own quanta. Past
  `CATCHUP_TICKS` of backlog (a lag spike) it runs double rate; a client
  silent for `STARVE_TICKS` gets its car driven on with the held control,
  unacked.
- **Ack + rewind / replay.** `AgentState::ack` is the number of the
  player's quanta in the snapshot. The client keeps its last
  `Prediction::HISTORY` predicted quanta and compares its prediction *at the ack*
  with the server state. In agreement nothing happens; otherwise it rewinds
  to the server state and replays the inputs since the ack.
- **Ghosts.** Remote cars are carried along in the replay. A `Ghost`
  starts at the remote car's snapshot pose on the local quantum of that
  snapshot (the ack) and runs full physics per local quantum with its
  snapshot traction / rudder held (up to `GHOST_LEAD_MAX` quanta), with
  `collide_cars` against the local car on both bodies, the way the server
  does it. With a ghost within `GHOST_REPLAY_RANGE` every snapshot replays,
  so a ram is predicted from the server's state of both cars rather than
  against a remote car frozen in place.
- **Home tile.** The server folds every car onto the home tile
  (`rem_euclid`) after each step and each contact; the predicting client
  does the same (`fold_home`) so both run the physics on identical float
  coordinates. Terrain lookups floor (not truncate) coordinates, so a car at
  a negative coordinate samples the right cell.
- **Render offset.** What a replay moves the car by goes into
  `RenderOffset`, drawn on top of the physics pose and decayed (time
  constant 0.1 s, at most 30 u/s and 1.5u per frame). Past 50u it fades
  out within 0.3 s (over at least 3 frames) instead; only a jump over 200u
  (a teleport) is shown at once. The camera follows the drawn pose.
- **Same ground.** Online clients don't cut wheel tracks, burrows, smashed
  ceilings or grader sweeps into their level (`terraform::drain_vehicle`):
  the server never edits its copy, and a client driving over its own tracks
  was predicted over different ground. The original game does deform the
  collision height map with tracks, in multiplayer too, but each of its
  clients was authoritative for its own car. Dust still flies.
- **Bumps.** A `bumps` change still makes the reconcile take the server's
  velocities (the replay starts from them anyway).
- **Protocol 2** (`PROTOCOL_VERSION`): `Join::protocol`,
  `Input { first_quant, quanta, control }` (was `{ sequence, control }`),
  `AgentState::ack`, `SetPose::next_quant` (drops queued input from before a
  debug pose). The server turns away clients on another revision.
- **Remote players** are drawn at their snapshots, interpolated at the
  20 Hz tick rate; a snapshot across the torus seam is moved next to the
  previous one so the car does not slide across the map.
- **Web** runs the same `ClientSync` per quantum: full controls in every
  frame's batch (motor, rudder, brake, turbo, roll and the jump on the
  last quantum, a released input while the car is held by a train ride or
  an escave screen), the same reconcile, ghosts, render offset and bump
  adopt, and draws the remote players with their own cars and colours.
  Single player is unchanged.

Try it under a real-world ping on localhost with the server's simulated lag
(held each way, so the round trip is twice the latency plus jitter):

```bash
cargo run --release -p vangers-server -- --port 7800 --ws-port 7801 --sim-latency-ms 100 --sim-jitter-ms 30
cargo run --release --bin road -- --server 127.0.0.1:7800 --name Alice
```

`tests/lag_compensation.rs` runs client, server and link deterministically
(1-5 ticks of latency plus jitter, 17 ms / 200 ms / random frames; drive,
jump and a ram) on the test level and, when the data is present, on
Fostral. It asserts a visible per-frame correction under 2u, no snaps and
convergence with prediction, and prints what the old reconcile does on the
same runs. The "shared" runs drive the client through `ClientSync` /
`RemoteCar` (what native and web run) and must hold the same bounds and
drive the same path; `shared_client_first_sync_from_a_cold_start` starts
it 300u off the server's spawn, unsynced, as the web build does.

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

## Spawn at (roof / camera tests)

`--spawn-at X,Y` places the first **fresh** joiner at map `X,Y`, 5u above the
top surface there (on a roof, not in the cave under it), facing −Y. It
overrides `--duel-spawn` for joiner 0 only; joiner 1 keeps its duel place,
later joiners use the regular grid, and a reconnect within the grace window
still restores its last pose.

On Fostral, `1030,3985` is the broad roof (top 78–81) over the rock channel
east of the arch, with the arch crest ~60u west and the underpass below; the
car settles there upright within ~1u (`roof_spawn_stays_on_the_roof` in
`tests/chase_camera.rs`). `1010,3975` and `1020,3960` hold as well. The
arch's own crest (x 964–975) is too narrow for a mechos: a car dropped on
`970,3960` slides off into the sand.

```bash
cargo run -p vangers-server -- --port 7800 --ws-port 7801 --level Fostral --spawn-at 1030,3985
# with the duel pair: Alice on the roof, Bob at his duel spot
# cargo run -p vangers-server -- ... --duel-spawn --spawn-at 1030,3985
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

The web client runs the same `ClientSync` and logs the same lines to the
browser console (`Server correction ...: shown at once`, `Server bump: ...`).

### Web debug overlay

Without DevTools, the web page shows the same lines in a **Net debug**
window at the top right: the predicted pose of the local car (`predicted`,
with the next quantum number), the newest server pose (`server`, with its
ack), what that snapshot did (`InSync`, `Replayed(n), moved ...u`, `held`),
and the last 8 join / leave / `Server correction` / `Server bump` / train
ride / escave lines, stamped in UTC like the server log. It is on by default
when the page joins a server; `#debug=1` shows it solo, `#debug=0` hides it,
and F2 (or the checkbox in Settings) toggles it.

The server logs each car's pose every 5 s to compare against it:

```
Pose p2 (Bob) pos=(640.6,130.2,57.0) speed=0.1 ack=2983
```

### Controls on the wire (server)

The server logs the brake, turbo and jump it receives from each client: a
jump every time, held brake/turbo at most once a second per player.

```
Input p2 (Bob) jump power=1.4 at quant 2890
Input p2 (Bob) brake=0 turbo=9 (quanta held since the last line)
```

On the web page, jump is Alt (as in the native game) or J; brake is Ctrl,
turbo Shift. A tap shorter than a frame still counts, which matters at one
frame a second.

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
# shared client prediction (ClientSync / RemoteCar):
cargo test --lib physics::client
# lag compensation (prediction vs the old reconcile, simulated latency):
cargo test --release --test lag_compensation -- --nocapture
# server integration (incl. SetPose / SetSpiral → WorldState, reclaim, duel spawn):
cargo test -p vangers-server --test integration
```

See the header comment in `tests/net_physics.rs` for how to extend that
integration test with `CycleState` / `MovingLandState` round-trips.
