# Verification and limitations

ssbm-rs measures function behavior, replay state, GPU output, and audio separately. A playable build is not a claim that every path or platform has been verified.

## Recorded coverage

The local campaign report `report-f8.txt` from October 4, 2026 records 152,636 of 163,387 countable blocks verified by runs (93.4%), with another 5,069 explained by the gap ledger. These are campaign measurements, not a fresh result from building this checkout.

That report records no unmutated mismatches, ten reviewed input-dependent cases, and one mutated case awaiting review. The session handoff records a completed review of an earlier remaining case; report and handoff state must be reconciled before claiming no outstanding reviews. The report also flags 24 ledger entries whose blocks were subsequently verified. Those claims need removal from the ledger.

Results are counted across frozen binaries only where the function's port is unchanged. Harness corrections can invalidate earlier results independently of a port change. The [campaign workflow](../tools/lockstep/campaign/README.md) explains staleness, exclusions, and reviewed ledgers.

Raw captures, cards, and replay corpora stay outside Git because they can contain game data or player information. Published summaries should identify the source revision, binary revisions, coverage denominator, exclusions, and known exceptions. Local results are not reproduced by basic CI.

## What each check establishes

| Check | Reference | Scope and limits |
| --- | --- | --- |
| Floating-point differential checks | Vendored Dolphin interpreter code and a transcription of Ishiiruka's JIT | Instruction behavior; Slippi mode has documented comparison exceptions |
| Function lockstep | Original code executed by the Gekko interpreter | Returns and memory writes on exercised paths; relies on the interpreter and harness |
| Replay oracle | Slippi recordings | Recorded player state and RNG across settled frames; supported codes and float mode matter |
| Runtime memory comparisons | Dolphin snapshots | Independent runtime checks; a separate target from function coverage |
| GPU stream comparisons | Original-code and all-port runs | Draw and copy state; commands alone do not prove matching pixels |
| Renderer image comparisons | Dolphin's software renderer | Images within defined tolerance |
| AX differential checks | Dolphin's DSP HLE AX implementation | Command-list memory effects; does not establish DSP LLE sample equivalence |

The initial 40-replay smoke set matched the same recording comparisons as original-code runs. Twelve replays have documented entry-frame or off-screen-flag differences in local known-divergence lists. This is not evidence that a large rotating corpus or every Slippi code set has passed.

## Player limitations

The reference disc is Melee US NTSC 1.02, GALE01 revision 2. Other revisions and modded builds are unsupported. Replay patches outside translated code sets can retain original execution; supported playback must be established per code set.

The player still contains the interpreter. Complete interpreter removal, independent runtime validation, and release-suite results on all three target operating systems remain work.

The renderer has host-GPU approximations in rasterization, blending, and dithering. Logic operations and Z24 depth copies are listed as missing in the current tracker; indirect texturing has not yet been observed in Melee comparison frames. Lines and points lack some texture-offset behavior.

The image threshold is PSNR of at least 38 dB with no more than 0.25% of pixels differing by over 32 levels. This applies to the compared frames, not every possible scene. See [graphics and audio tools](../tools/gx/README.md).

## Basic CI and local verification

The basic CI workflow tests CPU, memory, runtime, disc loading, GX processing, and AX mixing without game data on Windows, macOS, and Linux. It provides infrastructure for future results, not evidence of a completed platform run until its jobs pass. Disc-backed tests skip when `SSBM_DISC` is unset.

CI does not run the player, GPU image suite, C++ differential fuzzers, replay corpus, or lockstep campaigns. Run relevant disc-backed checks locally and report their inputs and limitations. Never attach captures, disc bytes, cards, or personal replays to public issues or CI artifacts.
