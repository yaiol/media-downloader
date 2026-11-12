# Media Downloader

Media Downloader is part of [yaiol Applications](https://apps.yaiol.com).

Paste links, pick a preset (video in a given quality, or only the sound), and download them in a batch. A playlist becomes one row per video. It uses [yt-dlp](https://github.com/yt-dlp/yt-dlp) to download and [ffmpeg](https://ffmpeg.org) to merge the picture and the sound; the app installs both itself and keeps yt-dlp up to date.

## Install

Download the latest installer from the [releases page](../../releases/latest).

## Build from source

Requires the Rust toolchain (stable) and, for the installer, the Dioxus CLI (`cargo install dioxus-cli`).

```bash
cargo run                                         # run in development
cd app && dx bundle --desktop --release --package-types nsis   # Windows installer → release/
```

## Architecture

A Cargo workspace of two crates:

- `engine/` — everything that is not the window: pure logic, no UI dependency.
- `app/` — the Dioxus desktop shell. The look comes from the shared stylesheet in `app/assets/`;
  strings come from `locales/<lang>.json`, embedded in the executable at build time.
