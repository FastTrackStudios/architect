+++
title = "App settings"
description = "architect::settings — a typed Store<T> of facet JSON in the app's config dir (atomic, skip-if-equal, optional 0600), and FileKv."
weight = 62
+++

`architect::settings` (the `settings` feature) keeps an app's settings on
the device: one typed value per [`Store<T>`], stored as facet JSON.

```rust,ignore
use architect::facet::Facet;
use architect::settings::Store;

#[derive(Facet, Default, PartialEq)]
struct Prefs { theme: String, zoom: u32 }

let prefs: Store<Prefs> = Store::open("my-app", "prefs")?;
let mut p = prefs.load()?;   // Prefs::default() when nothing is stored
p.zoom = 120;
prefs.save(&p)?;             // Ok(false) when the stored body is already this
let token: Store<Token> = Store::open("my-app", "token")?.private(); // 0600
```

## Where it lives

The key is `"{name}.json"` on every target.

| target | location |
|---|---|
| native | `<app dir>/<name>.json` |
| wasm | `localStorage["<app>/<name>.json"]` |

The app dir is `settings::app_dir(app)`: `$<APP>_CONFIG_DIR` when set
(`my-app` → `MY_APP_CONFIG_DIR`; use it for tests, CI and portable
installs), otherwise the platform config dir plus the app name
(`~/Library/Application Support/my-app`, `$XDG_CONFIG_HOME/my-app`,
`%APPDATA%\my-app`). `Store::in_dir` takes an explicit directory.

## Write semantics

- **Atomic.** The body goes to a temp file in the same directory, is
  `fsync`ed, renamed over the target, and the directory is synced. A crash
  leaves the old file or the new one, never a torn one. Stale temp files
  from a dead writer (`.<name>.tmp-<pid>-<n>`) are never read.
- **Skip if equal.** When the file already holds the exact body, nothing is
  written: no rename, no mtime change, no file-watcher event. `save`
  returns whether it wrote.
- **Private.** `.private()` creates the file `0600` (unix) and tightens an
  existing one on the next save.
- **Bad bodies are errors.** A file that doesn't decode is
  `SettingsError::Decode`. It is never silently reset to the default.

## `FileKv`

`FileKv` is the directory-of-files key/value store under `Store` (native
only): key `k` is the file `<dir>/<k>`, any byte outside
`[A-Za-z0-9._-]` `%XX`-escaped. It has the same atomic, skip-if-equal
writes. With `atom-persist` also on, it implements architect-atom's `Kv`,
so `KvRows::new(FileKv::new(dir), "app/")` is a durable native
`LocalRows`. It holds the same keys `LocalStorageRows` writes in the
browser, without a database (see [optimistic](@/architecture/optimistic.md)).
