# Data-rights release gate

The engine is Apache-2.0. Fostral world data is licensed separately by
its rights holder.

## Executed grant

- [x] Rights holder: Association K-D Lab.
- [x] Published 2026-08-17 in `KranX/Vangers` commit `f1ad7d797b219eeae815a7f2b35b15787759040c`.
- [x] License: CC BY-SA 4.0. Attribution: *"Vangers — Fostral world data" by Association K-D Lab*.
- [x] Canonical source: https://github.com/KranX/Vangers/tree/master/data/thechain/fostral
- [x] Publication figures and the supplemental video use Fostral and carry that attribution.
- [x] Converted height layers are derived from that tree; the harness rebuilds them.
- [x] JCGT supplement does **not** re-host the world data. Readers fetch the pinned commit. CC BY-SA is ShareAlike; the journal asks for non-restrictive (MIT/BSD-like) licenses on hosted artifacts, so the canonical Git tree is the distribution channel.
- [x] Other Vangers worlds, music, character art, and trademarks are not covered.
- [x] The generated `output.vmt` cache is excluded upstream and is not used.

## Content scope

- [x] Fostral is named explicitly; pin `f1ad7d7`.
- [x] Fostral includes `harmony.pal` and the files `convert` needs.
- [x] The other nine survey worlds are **not** in the grant. §6.2 states that those rows require a lawfully obtained game copy.
- [x] `docs/assets/original.jpg` is a screenshot of the original *Vangers* software renderer. Caption attributes Association K-D Lab.

## Release check

- [x] Paper Data availability text matches these terms.
- [x] `tools/compare-terrain.py` fetches the pinned commit for Fostral.
- [x] Do not upload `fostral.zip` as a JCGT supplemental file.
