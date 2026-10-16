# tauri-plugin-dragout

[![Crates.io](https://img.shields.io/crates/v/tauri-plugin-dragout.svg)](https://crates.io/crates/tauri-plugin-dragout)
[![CI](https://github.com/alexqqqqqq777/tauri-plugin-dragout/actions/workflows/ci.yml/badge.svg)](https://github.com/alexqqqqqq777/tauri-plugin-dragout/actions/workflows/ci.yml)

**Let users drag files _out_ of your Tauri 2 app**: onto Finder, the Desktop, Mail,
Slack, Telegram or any other app. That works for files on disk, for bytes your
frontend just generated, and for files that only come into existence when the user
drops them.

```js
import { draggable } from 'tauri-plugin-dragout-api';

draggable(tile, { type: 'file', path: '/Users/me/Documents/contract.pdf' });
```

## The problem it solves

Users expect to grab a document in your app and drop it wherever they need it. In a
Tauri app on macOS that silently does nothing:

* **The webview cannot do it.** An HTML5 `dragstart` in WKWebView carries only text,
  URLs and in-page data. Chrome's `DownloadURL` trick does not exist in WebKit.
* **Tauri core only handles the opposite direction**, files dropped *into* a window.
* **The usual workarounds hurt.** Users get a "Save as…" dialog, then Finder, then a
  second drag. Or you write temporary copies to disk before every drag "just in case".
* **Often the file does not exist yet.** It may sit inside an archive, an encrypted
  vault or a database. It may be a report still to render, or a file still to
  download. macOS handles this with *file promises*: the app writes the file only
  after the drop, straight into the folder the user chose. Using that from Rust means
  Objective-C runtime classes, delegate protocols, the Blocks ABI, main-thread rules
  and mouse-event quirks.

This plugin hides all of that behind one call.

Typical uses:

* archive managers that extract on drop;
* password managers and encrypted vaults that decrypt on drop;
* document tools, editors and image apps that export the current state on drop;
* mail and chat clients that let users drag attachments out;
* cloud drives that download on drop;
* dev tools that let users drag logs, screenshots or exports out.

## Why dragout

* **Files that do not exist yet.** Other drag plugins can only drag a path that is
  already on disk. With dragout, nothing is written until the user drops, and then
  the file goes straight into the folder they chose. There are no temporary copies
  and no "Save as…" dialog.
* **No Rust needed for generated content.** A Blob, a canvas or bytes from your
  frontend become a real file on drop.
* **Your app learns where the file landed.** The `dropped` event carries the final
  path, so you can show "Saved to Desktop" or open the folder.
* **Slow work is fine.** Promises run on a background queue, so an extract, a decrypt
  or a download does not freeze the UI. Finder shows the file as in progress.
* **Safe by default.** A path filter limits what the frontend may drag, and the drag
  cannot land back in your own window. Panics and Objective-C exceptions become
  errors instead of crashing the app.
* **One call, one code path.** `draggable(el, item)` covers the usual gotchas, and
  TypeScript types are included. The crate builds on every platform, and
  `isSupported()` tells the frontend when to fall back.

## Features

| Item | What the drop target gets | When the work happens |
| --- | --- | --- |
| `file` | the existing file or folder | nothing to do |
| `copy` | a copy, optionally renamed | copied on drop |
| `data` | bytes from JS or Rust written as a file | written on drop |
| `promise` (Rust) | whatever your closure writes | your closure runs on drop, on a background queue |

* **Several items in one drag**, with a stacked preview.
* **System file icons**, or your own preview image. A canvas snapshot from JS works too.
* **Events** fire when each promised file has been written (with its final path), when the
  drag ends or is cancelled, and when a write fails.
* **No accidental self-drops.** By default the drag cannot land back in your own webview.
* **Path filter.** You can limit which files the frontend is allowed to drag.
* **Builds everywhere.** On Windows and Linux the plugin compiles. `isSupported()` returns
  `false` there, so one code path can serve all platforms.

Platform: **macOS 10.13+** (`NSDraggingSession` + `NSFilePromiseProvider`). Windows and
Linux are on the roadmap.

## Install

```toml
# src-tauri/Cargo.toml
[dependencies]
tauri-plugin-dragout = "0.2"
# or: tauri-plugin-dragout = { git = "https://github.com/alexqqqqqq777/tauri-plugin-dragout" }
```

```rust
// src-tauri/src/main.rs
tauri::Builder::default()
    .plugin(tauri_plugin_dragout::init())
```

```json
// src-tauri/capabilities/default.json
{ "permissions": ["core:default", "dragout:default"] }
```

JavaScript helpers (optional, plain ESM, no build step):

```bash
npm install github:alexqqqqqq777/tauri-plugin-dragout
```

## Use from JavaScript

`draggable()` handles the details: a small movement threshold, the left button only, and
switching off the webview's own HTML drag for that element.

```js
import { draggable, startDrag, onDropped, onEnded } from 'tauri-plugin-dragout-api';

// An existing file
draggable(el, { type: 'file', path: '/Users/me/report.pdf' });

// Several items, computed at drag time
draggable(row, () => selected.map((f) => ({ type: 'file', path: f.path })));

// Bytes generated in the browser: Blob, Uint8Array, ArrayBuffer or base64
draggable(exportButton, async () => ({
  type: 'data',
  fileName: 'notes.md',
  data: new Blob([editor.getMarkdown()]),
}), { iconData: previewCanvas });

// A renamed copy, made only if the user actually drops it
draggable(el, { type: 'copy', source: '/data/7f3a.bin', fileName: 'Invoice 2026-10.pdf' });

onDropped(({ name, path }) => console.log(`${name} saved to ${path}`));
onEnded(({ cancelled }) => cancelled && console.log('drag cancelled'));
```

To call `startDrag(items, options)` yourself, call it from a `pointermove` or
`pointerdown` handler while the button is still held.

Without a bundler (`withGlobalTauri: true`):

```js
window.__TAURI__.core.invoke('plugin:dragout|start_drag', {
  items: [{ type: 'data', fileName: 'hello.txt', data: btoa('hello') }],
});
```

Options: `icon` takes a path to an image, and `iconData` takes image bytes or a canvas.
`iconSize` defaults to 64. `allowSelfDrop` defaults to `false`.

## Use from Rust: lazy files

Promises are where native drag-out shines. Nothing is written until the user drops, and
then your closure writes straight into the destination. The closure runs on a background
queue, so it can take its time. Finder shows the file as in progress meanwhile.

```rust
use tauri_plugin_dragout::{start_drag, DragItem, DragOptions};

#[tauri::command]
fn drag_out_entry(window: tauri::WebviewWindow, archive: String, entry: String) -> Result<u64, tauri_plugin_dragout::Error> {
    let name = entry.rsplit('/').next().unwrap_or("file").to_string();
    let item = DragItem::promise(name, move |dest| {
        my_archive::extract_entry(&archive, &entry, dest).map_err(|e| e.to_string())
    });
    start_drag(&window, vec![item], DragOptions::default()).map(|s| s.id)
}
```

In the frontend, call this command the same way, from a pointer handler while the button
is held. `start_drag` can be called from any thread: it hops to the main thread itself.

Restrict what the frontend may drag:

```rust
tauri::Builder::default().plugin(
    tauri_plugin_dragout::Builder::new()
        .path_filter(|p| p.starts_with("/Users/me/Library/Application Support/MyApp"))
        .build(),
)
```

## Events

| Event | Payload |
| --- | --- |
| `dragout://dropped` | `{ session, name, path }`: a `copy`, `data` or `promise` item was written |
| `dragout://ended` | `{ session, operation, cancelled }`: the drag finished |
| `dragout://error` | `{ session, name, path, message }`: writing an item failed |

`session` matches the `id` returned by `startDrag`. A `file` item emits no `dropped` event,
because the target receives the original file.

## Good to know

* **The mouse button must be held** when the drag starts. Otherwise you get the error
  "no mouse button is pressed". Use `draggable()`, or call from `pointermove`.
* **Add `user-select: none`** to draggable elements so that text selection does not fight the drag.
* **The target decides the file name on collisions.** Your writer gets the full
  destination path. Write exactly there.
* **Some apps accept only real files**, not promises. Browsers and some Electron apps are
  examples. For those, use `file` items.
* **Security:** with `dragout:default`, the frontend can drag any absolute path it can
  name. If your webview loads remote content, set a `path_filter`.

## How it works

`start_drag` runs on the main thread and takes the current mouse event of the window.
If there is none, it synthesizes one at the cursor. It then starts an `NSDraggingSession`
on the window's content view:

* **`file` items** are plain `NSURL` pasteboard writers.
* **All other items** are `NSFilePromiseProvider`s with a shared delegate. The delegate
  supplies the file name and the type (UTI from the file extension).
* **Writes run on a background `NSOperationQueue`.** When a write finishes, the delegate
  reports success, or an `NSError`, back to the drop target.
* **Panics and Objective-C exceptions are caught** at the FFI boundary. They are reported
  as errors instead of crashing the app.

## Compared to other options

* **HTML5 drag and drop** cannot produce files outside the webview.
* **[`tauri-plugin-drag`](https://github.com/crabnebula-dev/drag-rs)** by CrabNebula focuses
  on dragging files that already exist on disk, on several platforms. Use dragout when the
  content is generated, encrypted, archived or remote and should materialise only on drop.
  It also fits when you want drop notifications with the final path.

## Roadmap

* Windows: `IDataObject` with `CFSTR_FILEDESCRIPTORW` / `CFSTR_FILECONTENTS` (virtual files) and `DoDragDrop`.
* Linux: GTK drag with `text/uri-list` for existing files, and XDS for promises.
* Opt-in move operation, and a drag preview rendered from the DOM element automatically.
* Releases on crates.io and npm for the JS helpers.

## Migrating from 0.1

0.1 was written for a single archive app and exposed `native_drag_out(archivePath, filePath)`.
In 0.2 the archive backend is gone. To extract on drop, wrap your extractor in a promise:

```rust
DragItem::promise(file_name, move |dest| extract(&archive, &entry, dest))
```

See [CHANGELOG.md](CHANGELOG.md) and the runnable demo in [`examples/minimal`](examples/minimal):

```bash
cd examples/minimal && cargo run
```

## License

MIT or Apache-2.0, at your option.
