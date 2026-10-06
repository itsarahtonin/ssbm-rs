# Progress dashboards

These are saved research snapshots, with their aggregate data and rendering sources preserved in the repository. They do not update as campaigns run and do not establish release readiness. See [verification and limitations](../verification.md) for the scope of the evidence and unresolved work.

| Dashboard | Snapshot and purpose |
| --- | --- |
| [Port Coverage Map](https://claude.ai/artifact/T1No6obYSvv81w5vVLpLkb) · [source](coverage.html) · [data](data/coverage.json) | 19,668 functions, 33 saved rounds through f10, exported on October 6, 2026 (UTC). Compare measured coverage, ledger explanations, and outstanding reviews. |
| [Graphics and Audio Ladder](https://claude.ai/artifact/Ppx3oVXq7qbkRck8tvcrZ4) · [source](graphics.html) · [data](data/graphics.json) | Saved October 3, 2026 at 00:10 by the original tracker. Metrics for a suite of 40 GX-stream scenarios and 41 frame/audio scenarios, plus renderer features and measured history. |

The dashboard links open the hosted pages, published from the HTML files here.

## Reading coverage

The round slider follows saved coverage history toward a fixed target of 100% measured verification. Green functions have every countable block verified and no unexplained mismatch, including unreviewed mutations. Partially verified functions keep their measured color. **Show ledger explanations**, off by default, adds blue stripes over the explained fraction and a separate accounted percentage. It never changes the measured percentage or hides mismatch indicators.

For example, `sysdolphin/baselib/debugconsole_main` contains 48 functions and 827 countable blocks. At f5, 194 blocks were verified and 633 explained, leaving zero unaccounted blocks. The default display shows 23.5% measured verification. Turning on explanations adds stripes for the remaining 76.5%; it does not turn the unit fully verified. The explanations concern crash-screen functions reached through panic/error handlers; this display audit checked the saved arithmetic and ledger attribution, without independently proving that call graph or every ledger claim. At f10 the split is the same, 197 verified and 630 explained.

History is reconstructed from saved per-function reports, identified by unit and address. Each round retains its counts and mismatch flags, but mutation reviews use `reviewed.txt` as it stood at the export's source revision. These are not immutable records of what had been reviewed on each historical date. Snapshot provenance lists available frozen-binary revisions and exclusion files, mapped into this repository's commit history; an unavailable historical manifest is explicitly marked. Raw instruction bitmaps and logs remain private. Click a unit or search for a function to inspect its counts, ledger category, claim offsets, rationale, shared context, and source links. The evidence panel also exposes assertion/unreachable block counts and the current lists of functions excluded as unreachable or SDK substitutions. Historical ledger copies were not saved: current claim text and scope lists are explicitly distinguished from historical counts.

“Unaccounted blocks” subtracts both measured and explained coverage; it is not a count of all unexecuted blocks. Outstanding mismatch reviews can block completion even when no blocks remain unaccounted. Any ledger claim whose blocks a run has since verified is flagged in the evidence panel; at f10 the ledger has none (see [verification](../verification.md)).

## Graphics

Graphics/audio status requires the full expected suite to pass, and audio also requires replay synchronization. Historical graphics commits are mapped to the corresponding source trees in this repository; the saved tracker measured revision `3a1229d2afa85ce6ce83ba51aeb2ecb2c779aa0f`. Its branch label describes that historical measurement, not the current checkout. Replay filenames have stable anonymous scenario labels within the graphics dataset. Game-frame images remain private, so the frame viewer preserves metrics and explains why images are unavailable. Renderer features and window status are saved tracker annotations, not new measurements made during this export.

## Regeneration and checks

The JSON files are sufficient to regenerate these HTML pages with Python's standard library. Run from the repository root:

```sh
python tools/lockstep/campaign/treemap.py f10 docs/progress/coverage.html --from-json docs/progress/data/coverage.json
python tools/lockstep/campaign/coverage_svg.py
python tools/gx/progress.py local/progress-render --from-json docs/progress/data/graphics.json
```

The graphics command writes `local/progress-render/index.html`; copy that HTML to `docs/progress/graphics.html` when updating the committed page. Snapshot rendering does not read private results, copy game images, or append history. Updating the evidence itself requires a separately reviewed export from the campaign state; changing a page's timestamp does not refresh its measurements. A coverage export from private CSV reports also reads the repository's ledger and scope lists. Use `--evidence-revision` only when those files match that commit; otherwise it labels the ledger as an unversioned working tree and omits commit links.

Lightweight regression checks exercise the generators and actual template logic without building or running the game:

```sh
python -m unittest discover -s tools/lockstep/campaign -p test_reports.py
node tools/lockstep/campaign/test_treemap.cjs
node tools/gx/test_progress.cjs
```

To view a checkout's pages before publishing them, serve this folder with `python -m http.server 8765 --bind 127.0.0.1 --directory docs/progress` and open `http://127.0.0.1:8765/`.

A Coverage Levers page, the instruction yield and cost of each campaign run, was a planning aid for the campaigns and is no longer published; `tools/lockstep/campaign/levers.py` still writes it locally from the campaign state.
