# Progress dashboards

These are saved research snapshots, with their aggregate data and rendering sources preserved in the repository. They do not update as campaigns run and do not establish release readiness. See [verification and limitations](../verification.md) for the scope of the evidence and unresolved work.

| Dashboard | Snapshot and purpose |
| --- | --- |
| [Port Coverage Map](coverage.html) · [data](data/coverage.json) | 19,668 functions, 31 saved rounds through f8.8, plus the separately labeled working report exported on October 4, 2026 (local date). Compare measured coverage, ledger explanations, and outstanding reviews. |
| [Coverage Levers](levers.html) · [data](data/levers.json) | 204 recorded runs, their instruction yield and cost, and the coverage timeline. Gains use the available f7.10 baseline. |
| [Graphics and Audio Ladder](graphics.html) · [data](data/graphics.json) | Saved October 3, 2026 at 00:10 by the original tracker. Metrics for a suite of 40 GX-stream scenarios and 41 frame/audio scenarios, plus renderer features and measured history. |

Open [index.html](index.html) from a downloaded checkout in a browser. GitHub displays HTML source rather than running these pages. The coverage and levers pages load D3 7.9.0 from cdnjs; their charts need an internet connection. Alternatively, serve this folder locally:

```sh
python -m http.server 8765 --bind 127.0.0.1 --directory docs/progress
```

Then open `http://127.0.0.1:8765/`. Nothing is uploaded by this command.

## Reading coverage

Milestone **3a** requires at least 90% of a function's countable blocks verified by lockstep. Milestone **3b** requires every countable block either verified or accounted for by the gap ledger. Both exclude functions with unexplained mismatches, including unreviewed mutations. Under 3b, green means fully verified; blue means completion also relies on ledger explanations. Completion under 3b can therefore precede reaching 90% verified coverage under 3a.

For example, `sysdolphin/baselib/debugconsole_main` contains 48 functions and 827 countable blocks. At f5, 194 blocks were verified and 633 explained, leaving zero unaccounted blocks. The earlier display colored this completion green; it is now blue. The explanations concern crash-screen functions reached through panic/error handlers; this display audit checked the saved arithmetic and ledger attribution, without independently proving that call graph or every ledger claim. At f8.8 the split is 197 verified and 630 explained.

History is reconstructed from saved per-function reports, identified by unit and address. Each round retains its counts and mismatch flags, but mutation reviews use `reviewed.txt` as it stood at the export's source revision. These are not immutable records of what had been reviewed on each historical date. Snapshot provenance lists available frozen-binary revisions and exclusion files, mapped into this repository's commit history; an unavailable historical manifest is explicitly marked. Raw instruction bitmaps, logs, and excluded-function evidence remain private.

The latest working report is a separate entry: it does not replace f8.8. “Unaccounted blocks” subtracts both measured and explained coverage; it is not a count of all unexecuted blocks. Outstanding mismatch reviews can block completion even when no blocks remain unaccounted. The documented 24 ledger claims whose blocks were subsequently verified still require reconciliation; this export does not resolve them.

## Levers and graphics

The old levers page requested `f7`, which has no saved report, and silently used the current report as its baseline. This export explicitly uses **f7.10** instead. Function gains are matched by unit and address, not by name alone. Per-run yield counts instructions from private bitmaps, whereas coverage totals count basic blocks; overlapping runs' yields cannot simply be added. Cost uses the log's final instruction summary and includes corpus collection when recorded. Small costs retain the log's million-instruction precision rather than rounding to zero before calculating yield. Dated findings are historical annotations, including any mention of jobs running at that time; old pull request references belong to the private predecessor repository.

Graphics/audio status requires the full expected suite to pass, and audio also requires replay synchronization. Historical graphics commits are mapped to the corresponding source trees in this repository; the saved tracker measured revision `3a1229d2afa85ce6ce83ba51aeb2ecb2c779aa0f`. Its branch label describes that historical measurement, not the current checkout. Replay filenames have stable anonymous scenario labels within the graphics dataset. Game-frame images remain private, so the frame viewer preserves metrics and explains why images are unavailable. Renderer features and window status are saved tracker annotations, not new measurements made during this export.

## Regeneration and checks

The JSON files are sufficient to regenerate these HTML pages with Python's standard library. Run from the repository root:

```sh
python tools/lockstep/campaign/treemap.py "working report" docs/progress/coverage.html --from-json docs/progress/data/coverage.json
python tools/lockstep/campaign/levers.py f7.10 docs/progress/levers.html --from-json docs/progress/data/levers.json
python tools/gx/progress.py local/progress-render --from-json docs/progress/data/graphics.json
```

The graphics command writes `local/progress-render/index.html`; copy that HTML to `docs/progress/graphics.html` when updating the committed page. Snapshot rendering does not read private results, copy game images, or append history. Updating the evidence itself requires a separately reviewed export from the campaign state; changing a page's timestamp does not refresh its measurements.

Lightweight regression checks exercise the generators and actual template logic without building or running the game:

```sh
python -m unittest discover -s tools/lockstep/campaign -p test_reports.py
node tools/lockstep/campaign/test_treemap.cjs
node tools/gx/test_progress.cjs
```

Earlier hosted versions remain available for comparison: [coverage](https://claude.ai/artifact/T1No6obYSvv81w5vVLpLkb), [levers](https://claude.ai/artifact/SANQTUxB7JV4kjD7kVP5ER), and [graphics/audio](https://claude.ai/artifact/Ppx3oVXq7qbkRck8tvcrZ4). They have not been updated with these display fixes.
