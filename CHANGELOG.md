# Changelog

## [Unreleased]

### Fixed

- Progress ring no longer jumps backwards (e.g. 50% → 32%). Folder scanning now runs on
  its own threads ahead of copying, so the totals are final within moments; until then
  the ring shows a spinner with the number of files found.
- Time remaining now works: it's estimated from the recent file and data rates (whichever
  finishes later), smoothed so it counts down steadily.

## [1.0.0] - 2026-10-22

First public release.

### Added

- Parallel copy engine with drive-aware thread tuning (SSD, HDD, network, `subst` drives).
- Small-file fast path, `CopyFileEx` for large and special files, separate closer threads.
- Explorer integration: **Copy with SFC**, **Paste with SFC**, **SFC: copy to folder...** and *Send to*.
- Window with drag-and-drop, live speed graph, progress ring, free space and recent destinations.
- Compact paste window that closes itself after a clean finish.
- `superfastcopy` command-line tool with `overwrite` / `skip-same` / `skip` modes.
- Installer with optional PATH entry, Windows 11 full right-click menu and Microsoft Defender exclusion.

[1.0.0]: https://github.com/boulbeba115/sfc-super-fast-copy/releases/tag/v1.0.0
