# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] – 2026-10-16

A rewrite: the plugin is no longer tied to an archive app and works as a general
drag-out plugin for Tauri 2.

### Breaking
* Removed the `native_drag_out(archive_path, file_paths, target_dir)` command and the
  `blitzarch_backend` feature. Use a Rust `DragItem::promise` to extract on drop.
* New commands `start_drag` and `is_supported`. Add `dragout:default` to your capability.

### Added
* Drag items: `file` (existing file or folder), `copy` (copied and optionally renamed on drop),
  `data` (bytes written on drop), and `promise` (Rust closure called on drop).
* Several items per drag with a stacked preview, system icons or a custom preview image
  (`icon`, `iconData`, `iconSize`).
* Events `dragout://dropped`, `dragout://ended`, `dragout://error`.
* `Builder::path_filter` to restrict which paths the frontend may drag.
* Rust API `start_drag(&window, items, options)`, callable from any thread.
* JavaScript helpers in `guest-js`: `startDrag`, `draggable`, `isSupported`, `onDropped`,
  `onEnded`, `onError`, with TypeScript types.
* A runnable example app in `examples/minimal`.
* CI on macOS, Linux and Windows.

### Fixed
* Promises reported success even though nothing was written when the archive feature was off.
* The drag-ended callback used a wrong selector and never fired.
* Promise files were written on the main thread. They now use a background queue.
* The drag used the key window instead of the window that called it.
* The drag failed when the current event was not a mouse event.
* Delegates and file names leaked.
* The crate failed to compile on non-macOS targets. It now compiles everywhere and reports `Unsupported`.
* The README showed an invoke call that could not work.

## [0.1.1] – 2025-08-05
### Changed
* Removed mandatory `blitzarch` dependency; now optional feature `blitzarch_backend` is empty by default.
* Added **Installation** section to README.
* Added GitHub CI workflows for build and publish.

## [0.1.0] – 2025-08-05
### Added
* Initial public release.
* Native macOS drag-out using `NSFilePromiseProvider`.
* Optional `blitzarch_backend` feature for on-the-fly extraction.
* Example usage in Rust and JavaScript.
