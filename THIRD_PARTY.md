# Third party provenance

Project code is offered under GPL-3.0-or-later; [LICENSE](LICENSE) contains that license. Upstream files retain copyright and SPDX notices. The project license does not replace upstream terms or establish rights to original Nintendo/HAL code.

## Melee decompilation

Ports, layouts, and symbols are based on [doldecomp/melee](https://github.com/doldecomp/melee). The configured generation revision is `af0423e32394cdade84de17376c96f928961c534` in `tools/cloud/setup.sh`. The decompilation does not state a license for the original game code.

The translator reads original assembly and compiler debug information. Instruction-derived implementations and constants are distinct from bundled runtime assets, but still require provenance review. Textures, sound, disc files, and runtime data tables are loaded from the user's disc.

## Dolphin

Floating-point behavior, GX formats, rendering behavior, and early AX mixing follow or adapt [Dolphin](https://github.com/dolphin-emu/dolphin), GPL-2.0-or-later. Relevant files identify these references.

- `tools/fp-fuzz/dolphin/master` and `common`: reference source at `5102a0339c2177575378107b76541e47cc52122d`.
- `tools/fp-fuzz/dolphin/ishiiruka`: excerpts from [Slippi's Ishiiruka](https://github.com/project-slippi/Ishiiruka), `60f7b63496fb6ec7b9180a04f16f3edc0ad89fe2`.
- `tools/ax-dolphin/dolphin`: AX reference source identified by the graphics/audio workflow as revision `13e41434`.

C++ shims and stubs isolate the references. See [fp-fuzz](tools/fp-fuzz/README.md) and [graphics/audio verification](tools/gx/README.md).

## Slippi

Playback codes and injection lists come from [project-slippi/dolphin](https://github.com/project-slippi/dolphin), `41a7a3a110ed52999486ae1901c8fbb9a63d4f13`, GPL-2.0-or-later. The bootloader comes from [slippi-ssbm-asm](https://github.com/project-slippi/slippi-ssbm-asm), `fcf47f10dc244152c2ebaa3a9dec142ea42243b7`, GPL-3.0.

See [Slippi data provenance](crates/slippi/data/README.md). `crates/game/src/patched.rs` combines instruction-derived game functions with supported playback codes; its provenance is broader than Slippi alone.

## Cargo dependencies

`Cargo.lock` records external versions. Their licenses remain applicable; this document covers directly vendored or adapted source. [tools/run/notices.py](tools/run/notices.py) lists the crates built into the player for a target, with their license files, and each release carries that list as `THIRD-PARTY-NOTICES` beside the GPL text; the release's tag is its corresponding source.
