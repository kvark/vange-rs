# Design north star

**vange-rs** is a remake of *Vangers* (1998): same worlds, mechouses, and feel, rebuilt with modern Rust and GPU (wgpu).

## Remake fidelity

Single-player behavior and content stay aligned with the open-source original at [KranX/Vangers](https://github.com/KranX/Vangers). Prefer matching that tree (and the shipped data formats) over inventing parallel systems. Solo play should still read as “the 1998 game, accelerated,” not a different fantasy that happens to use the assets.

When unsure on remake systems: check how Vangers does it, then port or approximate — don’t redesign the fantasy.

## Persistent multiplayer (forward differentiator)

The 1998 original never shipped lasting multiplayer. **Persistent multiplayer** is the intentional forward path for vange-rs: shared worlds, authoritative server, native TCP + web WS clients, without breaking remake fidelity for solo play.

Prioritize work that strengthens or unblocks multiplayer (sync, authority, reconnect, shared story/cycle, playable two-client loops). Further original-feature ports are welcome when they serve that goal or keep solo parity from regressing; otherwise deprioritize them behind the MP track.

See [MULTIPLAYER.md](MULTIPLAYER.md) for ports, binaries, and a minimal two-client runbook.
