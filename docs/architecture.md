# Architecture

ssbm-rs combines translated Rust functions with a runtime that preserves GameCube addresses and data layouts. The original code and each Rust port can therefore run from the same state for comparison.

## Calls and memory

Game state lives in 24 MB of big-endian main memory, with auxiliary RAM and hardware state maintained by the runtime and SDK layer. Pointers retain their original 32-bit addresses. Generated typed handles and accessors read and write that storage.

Calls dispatch by the original function address. The target can be a Rust game function, an SDK implementation, or the Gekko interpreter. Lockstep runs a port and its original from equivalent state and compares their returns and writes. Nested calls can be checked as well; hardware and SDK interactions are recorded and replayed so checking does not perform them twice.

The current player still includes the interpreter and uses original code for bootstrapping. Removing that dependency belongs to the release work; a Cargo release profile alone does not remove it.

## Translation

[c2rs](../tools/c2rs/README.md) translates C units from the matching decompilation into `crates/game/src/tu`. The original assembly decides evaluation order, fused operations, register behavior, and stack layout where the C alone is insufficient.

Assembly and selected compiler-dependent functions are transliterated from their instructions. Their Rust implementations retain explicit register operations and control-flow state machines. They execute as Rust; the interpreter remains a separate reference implementation.

Game code normally reaches state through generated accessors. Translation support and instruction-derived code also use addresses and registers directly where preserving original behavior requires it. Accessors are a useful boundary for future work, but do not make every module independent of the original layout.

Generated files stay committed so building and reading the port does not require rebuilding the decompilation. Fix the generator or a deliberate manual implementation rather than editing generated output. See [Contributing](../CONTRIBUTING.md).

## Floating point

`gekko-fp` implements the Gekko's arithmetic, conversions, estimates, paired singles, and NaN behavior. Game logic uses these operations rather than platform math where exact results matter.

`FpMode::Hardware` targets the console and is the default for play. `FpMode::Slippi` reproduces the older x86-64 Ishiiruka JIT used to record the supported replay corpus, which can differ from hardware in rounding and signed zero.

## Hardware, rendering, and audio

The SDK layer supplies virtual time, disc access, controllers, memory cards, and hardware interactions. These must be validated separately from translating game functions correctly.

`ssbm-gx` processes the GPU command stream and exposes draws and copies through host traits. `ssbm-render` transforms vertices and draws with wgpu. Command-stream comparisons and image comparisons measure different parts of fidelity; the host GPU introduces documented image approximations.

`ssbm-ax` implements the early AX audio microcode's behavior through a memory bus interface. Its differential checker uses Dolphin's DSP HLE AX implementation. Sample comparison against DSP LLE output remains a separate verification target.

## Verification boundaries

Function lockstep checks translated functions against the interpreter. Replay checks compare recorded state with Slippi recordings. Dolphin-based memory, image, and audio checks can expose errors shared by the runtime and its ports. Passing one level does not prove the others.

See [verification](verification.md) for the evidence and [reusable components](components.md) for boundaries that could support other projects.
