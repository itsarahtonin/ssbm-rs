# Verification and limitations

ssbm-rs measures function behavior, replay state, GPU output, and audio separately. A playable build is not a claim that every path or platform has been verified.

## Recorded coverage

The latest local campaign report, after round rf on October 14, 2026, records 153,412 of 163,387 countable blocks verified by runs (93.9%), with another 5,158 explained by the gap ledger (3.2%), leaving 4,817 open (2.9%). No ledger line covers a block runs have verified: round f12's mutated checks reached 42 blocks that lines called unreachable, and round rf's reached 7, and those lines were dropped. These are campaign measurements, not a fresh result from building this checkout.

Round rf rechecked the 384 ports that a fix to `tools/c2rs/regflow.py` regenerated. regflow had treated every call as setting the volatile registers. So code ported from machine code could read a register that a C-ported caller never wrote: Nana's CPU DI (`ftCo_800AC5A0`) passed a stale stick value in r5 under `--port all`. Registers are now followed through calls that leave them as they found them, and 397 callers and 345 callees are ported from machine code, up from 379 and 12. Rerunning the 156 earlier jobs whose results covered those ports re-verified all but 207 of the blocks they held before (153,619 verified after round f12 and the played sessions). The rest came from mutated checks, probes and corpus fuzzing that an exact rerun doesn't repeat. On an idle machine the regenerated ports run 4–7% slower.

### The open blocks

The gap ledger takes only code no run can reach: dead branches, developer-only code, code that runs once the game has failed, and code for hardware the SDK layer leaves out. A block that unusual state could reach stays open, even when no real game reaches it. A triage of the open blocks on October 13, 2026 moved what qualified into the ledger: the debug modes' and debug menus' code, which runs only at a debug level a retail disc never sets; `HSD_Panic`; and the printf features that no format string in the game uses. That added 82 blocks.

The rest are reachable code that no run has reached yet. `tools/lockstep/triage.py` sorts them by what stands in the way:

| Kind | Blocks | What reaching them takes |
| --- | ---: | --- |
| Directed | 2,980 | A value at a branch next to verified code that a directed mutated check could set |
| Deep | 1,474 | Reaching an open block before them first |
| Untraced | 301 | A compare whose value can't be traced back to a mutation target |
| Switch | 38 | An untaken case of a switch next to verified code |
| Uncalled | 24 | Marth's Dancing Blade, which no run played |

By area they are 869 blocks in fighters, 830 in game modes and scenes, 736 in HAL's library, 499 in stages, 454 in menus, 430 in items, 255 in trophies, 131 in effects, 317 in other game code, 218 in Nintendo's SDK and 78 in the C and runtime libraries. They are spread thin: 1,543 functions have open blocks, 1,108 of them only one or two. The largest groups are features no run plays far: the snapshot album (`mnsnap`, 111), trophy collection (`toy`, 184), event matches (`gmevent`, 79), the memory card scene (`gmscmemcard`, 67) and Big Blue (`grbigblue`, 84), along with the memory card library's error paths (`sysdolphin/baselib/card`, 155). Each takes a scenario of its own for tens of blocks, so the campaigns stopped here.

The report has no mismatch with real inputs. The last one, in `OSResumeThread`, came from a match the game never sets up: a Crazy Hand without Master Hand, which failed one of the game's own assertions, so the check met the assertion's crash path. The original code alone also fails that match. The runner no longer draws such line-ups, and that job's results (`nighthands13`) are quarantined. Every mismatch from changed inputs is reviewed in `tools/lockstep/reviewed.txt`: twelve functions, all from inputs the game cannot produce. None awaits review. `fn_8017D9C0`'s two mismatches, once reviewed as the two sides seeing different state, came from the probe harness passing arguments past r10 in registers, which overwrote r13 for three probed functions; probes now pass them on the stack, and those functions' earlier probe results no longer count.

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

Some online replays recorded with Slippi 3.16 can't be reproduced from the file, by ssbm-rs or by Slippi Dolphin with its resync turned off. That version's UCF code snaps a C-stick held near a cardinal to exactly 1.0 using the raw C-stick bytes, after the recorder has saved the frame's inputs, and 3.16 replays don't record those bytes (3.17 does). The difference first shows at the end of hitlag, where smash DI moves 3 times the C-stick and playback comes out 0.0375 short. In a sample of 61 online replays, 21 of the 51 recorded with 3.16 diverged this way.

The other divergences in that sample left gameplay unchanged. In some special moves, the field Slippi records as hitstun holds a heap pointer, which differs between the original game, Slippi Dolphin and ssbm-rs. Players who used a widescreen code, which playback leaves out, were on screen in the original where playback has them offscreen. And Melee reads controllers on a 1/60 s alarm while video runs at 59.94 Hz, so once every 1,001 frames it runs two game frames for one render. Because ssbm-rs gives CPU work no virtual time, its skipped renders fall on different frames than Dolphin's. The offscreen flag is only updated while rendering, so it can differ on those frames. Only the HUD magnifier reads it.

## Player limitations

The reference disc is Melee US NTSC 1.02, GALE01 revision 2. Other revisions and modded builds are unsupported. Replay patches outside translated code sets can retain original execution; supported playback must be established per code set.

The player still contains the interpreter. Complete interpreter removal, independent runtime validation, and release-suite results on all three target operating systems remain work.

The renderer has host-GPU approximations in rasterization, blending, and dithering. Logic operations and Z24 depth copies are listed as missing in the current tracker; indirect texturing has not yet been observed in Melee comparison frames. Lines and points lack some texture-offset behavior.

The image threshold is PSNR of at least 38 dB with no more than 0.25% of pixels differing by over 32 levels. This applies to the compared frames, not every possible scene. See [graphics and audio tools](../tools/gx/README.md).

## Basic CI and local verification

The basic CI workflow tests CPU, memory, runtime, disc loading, GX processing, and AX mixing without game data on Windows, macOS, and Linux. It provides infrastructure for future results, not evidence of a completed platform run until its jobs pass. Disc-backed tests skip when `SSBM_DISC` is unset.

CI does not run the player, GPU image suite, C++ differential fuzzers, replay corpus, or lockstep campaigns. Run relevant disc-backed checks locally and report their inputs and limitations. Never attach captures, disc bytes, cards, or personal replays to public issues or CI artifacts.
