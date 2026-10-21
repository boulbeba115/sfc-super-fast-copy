# Changelog

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
