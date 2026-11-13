<p align="center">
  <img src="docs/assets/logo.png" alt="Media Downloader" width="110" height="110">
</p>

<h1 align="center">Media Downloader</h1>

<div align="center">
  <strong>Paste links, pick a preset, download - a whole playlist at once.</strong><br>
  Video or just the sound, to the folders and file names you choose, with nothing to install: the app fetches its download engine itself and keeps it up to date.
</div>

<br>

<!-- readme:nav -->

<div align="center">
  <a href="../../releases/latest"><img src="https://img.shields.io/github/v/release/yaiol/media-downloader?color=5a4fff&label=release&style=flat-square" alt="Release"></a>
  <a href="../../releases"><img src="https://img.shields.io/github/downloads/yaiol/media-downloader/total?color=5a4fff&label=downloads&style=flat-square" alt="Downloads"></a>
</div>

<h3 align="center">
  <a href="https://apps.yaiol.com/p/media-downloader/">Website</a>
  <span>&nbsp;·&nbsp;</span>
  <a href="#install">Install</a>
  <span>&nbsp;·&nbsp;</span>
  <a href="#what-it-is">Features</a>
  <span>&nbsp;·&nbsp;</span>
  <a href="#documentation">Documentation</a>
  <span>&nbsp;·&nbsp;</span>
  <a href="#build-from-source">Development</a>
</h3>

<!-- /readme:nav -->

---

## Install

<div align="center">

<table align="center">
<tr>
<th align="center">Windows</th>
<th align="center">macOS</th>
<th align="center">Linux</th>
</tr>
<tr>
<td align="center"><a href="../../releases/latest"><img src="https://img.shields.io/badge/Windows-.exe-5a4fff?style=for-the-badge&amp;logo=windows&amp;logoColor=white" alt="Windows"></a></td>
<td align="center"><a href="../../releases/latest"><img src="https://img.shields.io/badge/macOS-.dmg-5a4fff?style=for-the-badge&amp;logo=apple&amp;logoColor=white" alt="macOS"></a></td>
<td align="center"><a href="../../releases/latest"><img src="https://img.shields.io/badge/Linux-.deb%20%7C%20.AppImage-5a4fff?style=for-the-badge&amp;logo=linux&amp;logoColor=white" alt="Linux"></a></td>
</tr>
<tr>
<td align="center">x64 installer</td>
<td align="center">Apple Silicon</td>
<td align="center">deb package or AppImage</td>
</tr>
</table>

</div>

> **Windows note:** SmartScreen may warn on first launch because the app is not code-signed. Click "More info", then "Run anyway".
>
> Windows is the platform the app is developed and tested on. The Linux packages are built from the same source on release and have had less use. On macOS the app starts but cannot yet install the tools it downloads with, so it cannot download there.

---

## What it is

Media Downloader is a window on top of [yt-dlp](https://github.com/yt-dlp/yt-dlp), the download engine that knows how to fetch from a very large number of video and music sites. You copy links in your browser, press one button to add them, choose what you want and where it goes, and press Download.

A link to a playlist or an album is opened into one row per item, so each can be removed, retried or played on its own. The window has two lists: links added to **Video** are downloaded as picture and sound, links added to **Audio** as sound only, and the same link can sit in both.

Nothing has to be set up first. The app installs yt-dlp, [ffmpeg](https://ffmpeg.org) (which joins the picture and the sound, and converts audio) and [Deno](https://deno.com) (which some video sites require) into a folder of its own, keeps them up to date, and never uses or changes a copy found elsewhere on the machine.

---

## Features

- **A batch from the clipboard** - copy one link or twenty and press +; text around the links is ignored.
- **Playlists and albums, one row per item** - remove what you do not want before downloading; one failure does not stop the rest.
- **Video presets** - a picture height limit and a file type (MP4, MKV, WebM), or the best the site has in whatever file holds it.
- **Audio presets** - MP3, M4A, Opus, or the sound exactly as the site stores it, with no conversion.
- **Destinations** - saved folders, each with its own file name pattern (`{title}`, `{uploader}`, `{year}`, ...) and, if you want, a sub-folder per album or playlist.
- **Repairs itself** - when a download fails, the app updates yt-dlp and tries once more before reporting an error.
- **Tagged audio** - title, artist, album, track number, genre, year and cover are written into the files.
- **Sign-in when a site asks** - borrows the login of a browser on the computer, or a cookies file you point it at, and only for the download that needs it.
- **A player** - play a finished file, a video with its picture, and step through the finished ones, without leaving the app.
- **Nothing lost on restart** - both lists come back as you left them.
- **Your language** - the interface is available in 52 languages.

---

## Documentation

| | |
|---|---|
| **User manual** | [Read it online](https://apps.yaiol.com/p/media-downloader/help/) |
| **What's new** | [Release notes](https://apps.yaiol.com/p/media-downloader/help/releases/) |
| **Product page** | [apps.yaiol.com](https://apps.yaiol.com/p/media-downloader/) |

---

## Build from source

Requires the Rust toolchain (stable) and, for the installer, the Dioxus CLI (`cargo install dioxus-cli`). The app drives **yt-dlp**, **ffmpeg** and **Deno**, which are not part of the repository and are not bundled: the app downloads them into a folder of its own the first time it starts.

```bash
cargo run -p media-downloader                                   # run in development
cargo test --locked                                             # engine and app tests (a few need the network or ffmpeg and are ignored by default)
cd app && dx bundle --desktop --release --package-types nsis    # Windows installer → release/
```

---

## Architecture

A Cargo workspace of two crates: the **engine** (everything that is not the window - installing and updating the tools, the presets, the destinations, looking a link up, the download queue and the player, with no UI dependency) and the **app** (the Dioxus desktop shell, which polls the engine every 200 ms; the engine never calls back into the window).

| Layer | Technology |
|-------|-----------|
| UI | Dioxus 0.7 on the system WebView |
| Download engine | yt-dlp, a separate executable the app downloads and installs itself |
| Merging and conversion | ffmpeg, installed the same way |
| Script runtime for some sites | Deno, installed the same way |
| Player | ffmpeg decodes; rodio plays the sound, the picture reaches the window as a motion-JPEG stream from a local server |
| Audio tags | lofty |
| Interface languages | flat JSON bundles embedded at build time |
| Packaging | `dx bundle` (NSIS, deb, AppImage, dmg) |

<details>
<summary><b>How a download runs</b></summary>

A link is first looked up: yt-dlp is asked what is behind it, and a playlist is opened into its items, so every row of the window is one file to fetch. Pressing Download hands the rows to a queue that runs a chosen number of them at once. Each job is one yt-dlp process, started with arguments built from the preset and the destination; its progress and the path of the finished file are read from its output. Lookups and downloads never overlap, because some sites refuse to answer both at once.

</details>

<details>
<summary><b>How the tools are kept up to date</b></summary>

Each tool is installed from its own project's releases, checked against the checksum published beside it, and recorded so the app only trusts a copy it installed. yt-dlp is always the latest release. The first failed download of a batch triggers an update check; if a newer yt-dlp is installed, every job that failed under the old one is retried once.

</details>

---

## License

The app downloads [yt-dlp](https://github.com/yt-dlp/yt-dlp), [ffmpeg](https://ffmpeg.org) and [Deno](https://deno.com) as separate executables from their own projects; they are not part of this repository or of the installer and keep their own licences.

<div align="center">
  <sub>Media Downloader is part of <a href="https://apps.yaiol.com">yaiol Applications</a>.</sub>
</div>
