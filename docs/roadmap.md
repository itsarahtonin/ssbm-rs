# Roadmap

This repository's goal is a faithful, verifiable Melee port and a durable record of how it was built. Its follow-on, melee-hd, lives in a separate repository.

## Current development

Active development paused in October 2026. The game functions have Rust implementations, the smoke replay set has been exercised with the ports, and a windowed player with rendering, audio, and controllers exists. The lockstep campaigns ended after round rf, which rechecked the ports a register-flow fix regenerated, with 93.9% of countable blocks verified, 3.2% explained by the gap ledger and 2.9% open. The open blocks are reachable code that no run has reached yet, mostly in features that need scenarios of their own. The repository stays useful as a reference: its runtime plays Slippi replays (the 3.14 to 3.17 and 3.19 recordings tried play to their end) and records state traces that other implementations can be compared with. Dated evidence and its limits are in [verification](verification.md).

The translation pipeline produces the bulk of the port. Earlier plans for assigning individual C files to manual porting tasks have been superseded by generator fixes, directed verification, and reviewed manual implementations.

## Complete the faithful port

- Verify every countable basic block through lockstep or explain it in the reviewed gap ledger, with no unexplained mismatches from real or mutated inputs.
- Remove ledger claims when subsequent runs verify those blocks, and keep the supporting evidence reviewable.
- Broaden replay coverage beyond the initial smoke set, retaining regressions and documenting compatibility exceptions.
- Continue directed checks for items, teams, multiple players, menus, memory cards, and single-player modes that competitive replays do not cover.
- Validate the runtime independently with Dolphin-based checks, including graphics and audio.

The verification target is every countable block checked by lockstep, with no unexplained mismatches. Ledger explanations account for some unverified blocks separately; they do not establish measured verification. An overall coverage percentage does not establish function equivalence.

## Prepare a release

- Replace the remaining original-code bootstrap and remove the interpreter from the player's build.
- Establish player-build and replay-suite results on Windows, macOS, and Linux.
- Validate controller polling, adapter behavior, and input latency.
- Document renderer approximations, unsupported features, and compatibility limits.
- Package the executable and corresponding source without disc data, extracted assets, captured memory, or personal replays.

Publishing development source and distributing a validated player release are separate milestones. Source provenance and third-party notices need review before publication; distributing binaries also needs a deliberate decision about the original game code's licensing.

## Preserve the research value

Keep the original-layout runtime, translator, floating-point implementation, interpreter, and verification methodology understandable as a reference. Document why compiler quirks matter and preserve meaningful verification history.

Reusable libraries may eventually move to their own repositories. [Component assessment](components.md) explains the candidates and what each needs before extraction.

## Separate modern Rust project

The follow-on project, melee-hd, started in October 2026 in a separate repository, which is not public yet.
