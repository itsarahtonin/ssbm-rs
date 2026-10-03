# The Graphics level

The Graphics rungs of the verification ladder, and the tools that check them. Progress is on the Graphics and Audio Ladder page (https://claude.ai/artifact/Ppx3oVXq7qbkRck8tvcrZ4).

## GX streams: original code against all ports

`ssbm-run`'s `GX_STREAM=FILE` logs the GPU's command stream frame by frame, each draw and copy digested by what decides its output (`ssbm-gx`), and `GX_DETAIL=N` logs frame N's draws one by one. `OUT=smoke bash tools/gx/smoke.sh` runs the 40 smoke replays both ways into `local/gx/smoke/` and compares each pair (`compare.py`); every remaining difference is to be fixed or explained.

## Frames: our renderer against Dolphin's software renderer

`ssbm-render` draws the stream with wgpu. `GX_DFF=FILE GX_DFF_FRAMES=START,COUNT` records a Dolphin FIFO log of a run, `gxplay LOG.dff OUT` draws it with our renderer, `dolphin-render.sh LOG.dff OUT N` with Dolphin's software renderer (on a hidden desktop, so nothing shows), and `gxplay diff OURS REF DIFFS` compares the two frame by frame. `OUT=frames bash tools/gx/frames.sh` does all of it for the smoke replays and a menu run into `local/gx/frames/`; delete the runs' `diff.txt` to compare again after a renderer change (Dolphin's frames are kept).

A frame is within tolerance when its PSNR is at least 38 dB and no more than 0.25% of its pixels are off by more than 32 levels. Dolphin's copy filter must be on (`User/Config/GFX.ini`, `[Enhancements] DisableCopyFilter = False`): Dolphin turns it off by default, hardware doesn't.

`GX_RENDER=DIR` renders a run's frames live into DIR. gxplay's `GXPLAY_TRACE`, `GXPLAY_SILHOUETTES`, `GXPLAY_OPAQUE` and `GXPLAY_DRAWS=N` help find what a frame draws wrong.

## The progress page

`python3 tools/gx/progress.py` writes `local/gx/progress/index.html` from the results above, with each run's worst frame (ours, Dolphin's and their difference) as full-size PNGs in `img/` beside it (listed in `files.txt`), and adds an entry to its history. Republish the page with its images (the Artifact `files`, rooted at `local/gx/progress`) to the page above after each change worth seeing. The renderer's feature list and the audio and window rungs' status are at the top of `progress.py`.
