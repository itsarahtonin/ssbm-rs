# The Graphics and Audio levels

The Graphics and Audio rungs of the verification ladder, the windowed build, and the tools that check them. Progress is on the Graphics and Audio Ladder page (https://claude.ai/artifact/Ppx3oVXq7qbkRck8tvcrZ4).

## GX streams: original code against all ports

`ssbm-run`'s `GX_STREAM=FILE` logs the GPU's command stream frame by frame, each draw and copy digested by what decides its output (`ssbm-gx`), and `GX_DETAIL=N` logs frame N's draws one by one. `OUT=smoke bash tools/gx/smoke.sh` runs the 40 smoke replays both ways into `local/gx/smoke/` and compares each pair (`compare.py`); every remaining difference is to be fixed or explained.

## Frames: our renderer against Dolphin's software renderer

`ssbm-render` draws the stream with wgpu. `GX_DFF=FILE GX_DFF_FRAMES=START,COUNT` records a Dolphin FIFO log of a run, `gxplay LOG.dff OUT` draws it with our renderer, `dolphin-render.sh LOG.dff OUT N` with Dolphin's software renderer (on a hidden desktop, so nothing shows), and `gxplay diff OURS REF DIFFS` compares the two frame by frame. `OUT=frames bash tools/gx/frames.sh` does all of it for the smoke replays and a menu run into `local/gx/frames/`; delete the runs' `diff.txt` to compare again after a renderer change (Dolphin's frames are kept).

A frame is within tolerance when its PSNR is at least 38 dB and no more than 0.25% of its pixels are off by more than 32 levels. Dolphin's copy filter must be on (`User/Config/GFX.ini`, `[Enhancements] DisableCopyFilter = False`): Dolphin turns it off by default, hardware doesn't.

`GX_RENDER=DIR` renders a run's frames live into DIR. gxplay's `GXPLAY_TRACE`, `GXPLAY_SILHOUETTES`, `GXPLAY_OPAQUE` and `GXPLAY_DRAWS=N` help find what a frame draws wrong.

## Audio: ssbm-ax against Dolphin's AX

`ssbm-ax` mixes the game's sound as Dolphin's DSP HLE runs the AX microcode. `AX=1` has the SDK's DSP stand-in run it (it's off by default: mixing updates the voices' parameter blocks, as hardware does, and the game reads them back), and `AUDIO_OUT=FILE.wav` records what the audio interface plays. `tools/ax-dolphin` builds Dolphin's own AX code from its source (vendored at 13e41434, with stubs for the rest of Dolphin); with ssbm-run built with `--features ax-check`, `AX_CHECK=1` runs every command list through both over the same memory and reports the first byte where what they write differs. `OUT=audio BIN=... bash tools/gx/audio.sh` checks every smoke replay whole and a menu run into `local/gx/audio/`, noting whether each replay still syncs with mixing on.

## Playing in a window

`cargo build --release -p ssbm-run --features window` builds `--window`: the game in a window at 59.94 fields a second, with sound, the first gamepad as controller 1 (A south, B west, X east, Y north, Z right shoulder, analog triggers for L and R, sticks and D-pad as they are), or the keyboard (arrows the stick, X A, Z B, C X, S Y, D Z, Q L, W R, Enter Start, IJKL the C-stick). With `--replay` the replay plays the controllers. `SSBM_MUTE=1` leaves the sound out; the rates the game and the window keep go to stderr every five seconds.

## The progress page

`python3 tools/gx/progress.py` writes `local/gx/progress/index.html` from the results above, with each run's worst frame (ours, Dolphin's and their difference) as full-size PNGs in `img/` beside it (listed in `files.txt`), and adds an entry to its history. Republish the page with its images (the Artifact `files`, rooted at `local/gx/progress`) to the page above after each change worth seeing. The renderer's feature list and the audio and window rungs' status are at the top of `progress.py`.
