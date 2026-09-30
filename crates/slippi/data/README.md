# Slippi data

Files from Project Slippi, used to set up replay playback exactly as Slippi Dolphin does:

- `playback.ini`: Slippi Dolphin's `Data/Sys/GameSettings/Playback/GALE01r2.ini` ([project-slippi/dolphin](https://github.com/project-slippi/dolphin) at `41a7a3a110ed52999486ae1901c8fbb9a63d4f13`, GPL-2.0-or-later), the Gecko codes playback runs.
- `bootloader.txt`: `Output/Bootloader/bootloader.txt` from [project-slippi/slippi-ssbm-asm](https://github.com/project-slippi/slippi-ssbm-asm) at `fcf47f10dc244152c2ebaa3a9dec142ea42243b7` (GPL-3.0), the codes that load the rest.

`src/denylist.rs` is generated from Slippi Dolphin's `Data/Sys/Slippi/InjectionLists` at the same commit.

These are Slippi's own codes, not game data.
