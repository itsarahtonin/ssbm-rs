# Roadmap

This repository's goal is a faithful, verifiable Melee port and a durable record of how it was built. A later idiomatic Rust project with modern features will live in a separate repository based on this one.

## Current development

The game functions have Rust implementations, the smoke replay set has been exercised with the ports, and a windowed player with rendering, audio, and controllers exists. Work is focused on verification coverage and closing or explaining remaining differences. Dated evidence and its limits are in [verification](verification.md).

The translation pipeline produces the bulk of the port. Earlier plans for assigning individual C files to manual porting tasks have been superseded by generator fixes, directed verification, and reviewed manual implementations.

## Complete the faithful port

- Verify every countable basic block through lockstep or explain it in the reviewed gap ledger, with no unexplained mismatches from real or mutated inputs.
- Remove ledger claims when subsequent runs verify those blocks, and keep the supporting evidence reviewable.
- Broaden replay coverage beyond the initial smoke set, retaining regressions and documenting compatibility exceptions.
- Continue directed checks for items, teams, multiple players, menus, memory cards, and single-player modes that competitive replays do not cover.
- Validate the runtime independently with Dolphin-based checks, including graphics and audio.

The first coverage milestone requires each function to meet the 90% coverage bar. The final function-verification gate requires every countable block to be verified or explained; an overall coverage percentage does not establish either milestone.

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

The follow-on repository would replace storage and object systems incrementally, using this faithful implementation as its behavioral reference. Netplay, training tools, and mod support belong there. Gameplay would remain identical by default, with intentional changes made explicit.

That repository has not been created. Its name, release policy, and extraction sequence remain to be decided.
