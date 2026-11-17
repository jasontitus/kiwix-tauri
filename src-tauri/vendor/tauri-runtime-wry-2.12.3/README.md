# tauri-runtime-wry

## Kiwix vendoring note

This is the crates.io `tauri-runtime-wry` 2.12.3 source package, upstream commit
`993038c49185307dfc895ab7657d77ca0d5ed7c7` (see `.cargo_vcs_info.json`), under its
original Apache-2.0 OR MIT licenses. The original registry package checksum is
`e1aa9d8b7a120f879f44fdc1053f7b7dbf416bf0bbc6a72b714dec46b782b52f`.
Registry bookkeeping and the upstream library lockfile are omitted.

The local source change in `src/lib.rs` limits GTK URI-scheme deduplication to
non-incognito webviews. Wry 0.57.1 creates a fresh ephemeral GTK context for every
incognito webview instead of using Tauri's cached context. Each fresh context
needs its own application and IPC scheme registrations, including after a window
is closed and recreated. Shared non-incognito contexts retain their existing
deduplication, and non-GTK platforms are unchanged. This preserves incognito
storage isolation without assigning persistent data directories or changing app
origins, navigation policy, archive access, or IPC authorization.

The regression requires a native GTK/WebKit replay: load the primary window,
open secondary windows from both native and frontend menus, use their independent
reader sessions and IPC, close and recreate a secondary window, and quit normally
with every window acknowledging its state flush. A source-wiring assertion would
not exercise the distinct WebKit contexts responsible for this failure.

 <img align="right" src="https://github.com/tauri-apps/tauri/raw/dev/.github/icon.png" height="128" width="128">

[![status](https://img.shields.io/badge/Status-Beta-green.svg)](https://github.com/tauri-apps/tauri)
[![Chat Server](https://img.shields.io/badge/chat-on%20discord-7289da.svg)](https://discord.gg/SpmNs4S)

[![test core](https://img.shields.io/github/actions/workflow/status/tauri-apps/tauri/test-core.yml?label=test%20core&logo=github)](https://github.com/tauri-apps/tauri/actions/workflows/test-core.yml)
[![website](https://img.shields.io/badge/website-tauri.app-purple.svg)](https://tauri.app)

[![https://good-labs.github.io/greater-good-affirmation/assets/images/badge.svg](https://good-labs.github.io/greater-good-affirmation/assets/images/badge.svg)](https://good-labs.github.io/greater-good-affirmation)
[![support](https://img.shields.io/badge/sponsor-Opencollective-blue.svg)](https://opencollective.com/tauri)

| Component         | Version                                                                                                                |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------- |
| tauri-runtime-wry | [![](https://img.shields.io/crates/v/tauri-runtime-wry?style=flat-square)](https://crates.io/crates/tauri-runtime-wry) |

## About Tauri

Tauri is a polyglot and generic system that is very composable and allows engineers to make a wide variety of applications. It is used for building applications for Desktop Computers using a combination of Rust tools and HTML rendered in a Webview. Apps built with Tauri can ship with any number of pieces of an optional JS API / Rust API so that webviews can control the system via message passing. In fact, developers can extend the default API with their own functionality and bridge the Webview and Rust-based backend easily.

Tauri apps can have custom menus and have tray-type interfaces. They can be updated, and are managed by the user's operating system as expected. They are very small, because they use the system's webview. They do not ship a runtime, since the final binary is compiled from rust. This makes the reversing of Tauri apps not a trivial task.

## This module

This crate opens up direct systems-level interactions specifically for WRY, such as printing, monitor detection, and other windowing related tasks. `tauri-runtime` implementation for WRY.
None of the exposed API of this crate is stable, and it may break semver
compatibility in the future. The major version only signifies the intended Tauri version.

To learn more about the details of how all of these pieces fit together, please consult this [ARCHITECTURE.md](https://github.com/tauri-apps/tauri/blob/dev/ARCHITECTURE.md) document.

## Semver

**tauri** is following [Semantic Versioning 2.0](https://semver.org/).

## Licenses

Code: (c) 2021 - The Tauri Programme within The Commons Conservancy.

MIT or MIT/Apache 2.0 where applicable.

Logo: CC-BY-NC-ND

- Original Tauri Logo Designs by [Daniel Thompson-Yvetot](https://github.com/nothingismagick) and [Guillaume Chau](https://github.com/akryum)
