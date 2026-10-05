# Reusable components

Several parts of ssbm-rs model GameCube behavior rather than Melee gameplay. Their crate boundaries let them remain readable and testable while a possible extraction is evaluated. This architectural assessment does not claim validation against other games.

| Component | Existing boundary | Assessment |
| --- | --- | --- |
| `gekko-fp` | No runtime or game dependency; `num-bigint` is used by tests | Strongest first candidate for interpreters, recompilers, and exact ports. Preserve hardware/Slippi modes, provenance, and differential tests. |
| `ssbm-gx` | No crate dependencies; host `Memory` and `Sink` traits | Promising command decoder. The FIFO writer currently hardcodes GALE01 metadata; broader command-stream validation is needed. |
| `ssbm-render` | GX and host rendering libraries | Could accompany GX. Document feature limits, device requirements, and non-Melee validation before presenting it as a general renderer. |
| `ssbm-ax` | No crate dependencies; host `Bus` trait | Promising early-AX implementation. Supported microcode variants are narrower than a general DSP emulator. |
| Memory, runtime, interpreter | Dependency chain through memory, context, and CPU helpers | Useful together as a porting and differential-check framework. Define the embedding API and capture format before splitting this group. |
| `ssbm-disc` | nod, memory, and Melee identity/hash constants | Melee-specific wrapper. Reuse nod for generic disc-container support. |
| c2rs and typegen | Decomp layouts, MWCC listings, build metadata, curated exceptions | Valuable research tools; generalization needs configurable boundaries and independent examples. |
| SDK, types, game, runner, campaigns | Game symbols, scenes, patches, and input recipes | Keep here; implementations are closely tied to Melee and its verification state. |

## Recommended sequence

Keep the workspace intact for the first public source preview. Add crate usage examples and document dependencies before introducing additional repositories and versioned releases.

If another consumer needs the arithmetic, extract `gekko-fp` first. Establish independent build and test inputs, preserve useful source history and upstream notices, then depend on a pinned release. Audit extracted history as carefully as the main repository.

Evaluate GX/renderer and early AX next, with an independent consumer or non-Melee fixtures exercising the proposed generic API. Keep the memory/runtime/interpreter group together until its stable boundaries are clear.

Extraction and crate publishing are future decisions. This assessment does not rename crates, change dependencies, or create repositories.
