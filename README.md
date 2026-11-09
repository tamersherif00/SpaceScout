# SpaceScout

**Disk space analyzer for Windows 10 and 11.** SpaceScout identifies the files and folders consuming
storage on any drive and lets you remove them directly from the results.

[![Release](https://img.shields.io/github/v/release/tamersherif00/SpaceScout)](https://github.com/tamersherif00/SpaceScout/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-lightgrey)

![SpaceScout folder tree view](docs/tree.png)

## Overview

When a Windows drive runs low on space, File Explorer offers no direct way to see which folders
are responsible. SpaceScout scans a drive or folder in parallel across all CPU cores, aggregates
sizes up the directory hierarchy, and presents the result as a size-ordered tree together with
ranked lists of the largest files and folders.

SpaceScout is distributed as a single portable executable of approximately 4 MB. It requires no
installation, runtime, administrator rights, background service, or network access.

It is intended as a lightweight, open-source alternative to tools such as WinDirStat, WizTree,
TreeSize and SpaceSniffer.

## Key capabilities

| Capability | Description |
|---|---|
| Parallel scanning | Work-stealing directory traversal across all available cores. A 460 GB system drive containing 4.3 million files is scanned in approximately 20 seconds with a warm file-system cache (approximately 70 seconds cold) on the reference machine. |
| Hierarchical tree view | Folders and files sorted by size, with hierarchy guide lines, size, percentage of parent and file count for every entry. The largest path is expanded automatically after each scan. |
| Ranked lists | The 1,000 largest files and 1,000 largest folders across the scanned location, each linked back to its position in the tree. |
| In-place deletion | Selected items can be moved to the Recycle Bin or deleted permanently. Totals are updated immediately without rescanning. |
| Safeguards | Confirmation listing every item and the total size; explicit warning when Windows, Program Files or other system paths are selected; the scan root cannot be deleted. |
| Drive overview | All mounted drives with free and total capacity, highlighted above 75% and 90% utilisation. |
| Explorer integration | Context menu actions to show in Explorer, open a folder, copy a path, or rescan a single folder. |
| Privacy | Fully offline. No accounts, telemetry or network calls. |

![SpaceScout largest files view](docs/largest-files.png)

## Installation

1. Download `spacescout.exe` from the [latest release](https://github.com/tamersherif00/SpaceScout/releases/latest).
2. Run the executable. No installation step is required.

Release binaries are not yet code-signed. Windows SmartScreen may therefore display a warning on
first launch; select **More info**, then **Run anyway**. Alternatively, build from source as
described below.

## Usage

1. Select a drive from the drive bar, or enter or browse to a folder, and choose **Scan**.
2. Expand folders using the arrow or by double-clicking. Each row shows size, share of the parent
   folder and the number of files contained.
3. Use the **Largest files** and **Largest folders** tabs for a ranked view across the whole scan.
   Double-click an entry to locate it in the tree.
4. Select items with the checkboxes and choose **Move to Recycle Bin** or **Delete permanently**.
5. Right-click any entry for Explorer and path actions.

Run SpaceScout as administrator to include locations that the current account cannot read, such
as other user profiles. The number of inaccessible folders is reported after each scan.

### Command line

```
spacescout.exe [PATH]            Open and immediately scan PATH
spacescout.exe --bench [PATH]    Scan without a window and print totals, duration and largest files
```

## Common sources of disk usage

| Location | Description | Recommended action |
|---|---|---|
| `pagefile.sys`, `hiberfil.sys` | Virtual memory and hibernation files | Do not delete. Resize the page file in System Properties, or run `powercfg /h off` to disable hibernation. |
| `*.vhdx` (WSL, Docker, Hyper-V, emulators) | Virtual machine disk images | Remove through the owning tool, for example `wsl --unregister` or Docker's data purge. |
| Downloads (`*.iso`, `*.zip`, installers) | Archives and installers | Generally safe to remove. |
| `node_modules`, `target`, `bin`, `obj`, `.gradle` | Build output and dependencies | Safe to remove; regenerated on the next build. |
| Package caches (uv, pip, npm, NuGet, Gradle) | Downloaded package caches | Clear with the tool's own command, for example `uv cache clean` or `npm cache clean --force`. |
| `AppData\Local\Temp`, `*.dmp` | Temporary files and crash dumps | Safe to remove. |
| `C:\Windows`, `Program Files` | Operating system and installed applications | Do not delete manually. Use Settings > Apps or Disk Cleanup. |

## Technical notes

- **Sizes** are logical file sizes, not allocated size on disk.
- **Hard links** (for example in `C:\Windows\WinSxS`) are counted once per link, so system folders
  may appear larger than their actual footprint.
- **Symbolic links and junctions** are not followed, preventing double counting and traversal loops.
- **Memory usage** is approximately 60–80 bytes per scanned entry; a 4.3-million-entry drive
  requires roughly 400 MB while displayed.

### Architecture

- **Scanner:** a parallel directory walk built on [rayon](https://github.com/rayon-rs/rayon). File
  sizes are read from the Win32 directory enumeration itself, avoiding a separate metadata call per file.
- **Data model:** results are flattened into a contiguous arena of 32-byte nodes with a shared
  string pool for names.
- **User interface:** built with [egui](https://github.com/emilk/egui) and virtualised, so only
  visible rows are laid out and directories with hundreds of thousands of entries remain responsive.

## Building from source

Prerequisites: [Rust](https://rustup.rs/) stable with the MSVC toolchain.

```
git clone https://github.com/tamersherif00/SpaceScout.git
cd SpaceScout
cargo test --release
cargo build --release
```

The executable is written to `target\release\spacescout.exe`.

## Roadmap

- Treemap visualisation
- Breakdown by file type
- Duplicate file detection
- CSV export
- Code-signed releases

## Contributing

Issues and pull requests are welcome. Please open an issue to discuss significant changes before
submitting a pull request.

## License

Released under the [MIT License](LICENSE). Copyright (c) 2026 Tamer Sherif.
