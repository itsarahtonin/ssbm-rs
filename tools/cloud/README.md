# Private campaign data

These scripts move local verification inputs between authorized development machines. They are not part of the public build or CI setup.

The data repository must remain private and separate from any public source repository. A release called `private-data`, or marked as a prerelease, has no independent privacy setting.

## Restore a campaign

Provide the private repository and the replay folder used by the bundled job lists:

```sh
GH_REPO=OWNER/PRIVATE_DATA_REPO REPLAYS_FROM=/original/replay/folder bash tools/cloud/setup.sh
```

`gh` must have read access. The script checks repository privacy before creating its data directory or downloading. `RELEASE` defaults to `private-data`; `SSBM_DATA` overrides the local download directory. Existing bundles can contain absolute replay paths, so `REPLAYS_FROM` is required to map them to the restored folder.

Setup restores the disc, replays, cards, generated inputs, and state; configures the pinned decompilation; writes ignored `local/env.sh`; and builds the runner. It can download substantial data and start a build. Run it deliberately.

## Pack local inputs

```sh
OUTDIR=/outside/the/source/repository bash tools/cloud/pack.sh
```

The archives contain game data and player information. Keep them outside source control and public artifacts. Packaging does not upload them.

Before making a source repository public, preserve needed data in its separate private destination and verify access there, then remove any data release/assets from the source repository. Audit other releases, tags, branches, and artifacts before changing visibility. Those remote changes require a separate publication decision.
