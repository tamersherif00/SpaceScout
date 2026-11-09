# SpaceScout

Fast, lightweight disk space analyzer for Windows (single 3.8 MB exe, no runtime needed).

- Pick a drive (bar shows how full it is) or any folder, then **Scan**. Uses all CPU cores in parallel.
- **Folder tree**: everything sorted biggest-first; expand with the arrow or double-click.
- **Largest files / Largest folders**: top 1000 across the whole scan. Double-click to reveal in the tree.
- Tick boxes to select, then **Move to Recycle Bin** or **Delete permanently** (with confirmation).
  Sizes update instantly without a rescan.
- Right-click any row: Show in Explorer, Open folder, Copy path, Scan only this folder.

Run as administrator to include folders your user can't read (e.g. other users' profiles, `System Volume Information`).

Notes: sizes are logical file sizes; symlinks/junctions are skipped (no double counting);
NTFS hard links (e.g. inside `C:\Windows\WinSxS`) are counted once per link, so Windows can look bigger than it really is.

## Build
```
cargo build --release      # -> target\release\spacescout.exe
cargo test --release
spacescout.exe "D:\Some Folder"      # start scanning a path immediately
spacescout.exe --bench C:\           # headless timing run
```
