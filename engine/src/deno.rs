//! Installing and updating Deno, the JavaScript runtime yt-dlp needs to read most YouTube videos.
//!
//! The same rule as yt-dlp (see `ytdlp`): the app never uses a Deno it did not install, never ships one, and keeps ONE copy, the
//! latest, in the folder the yaiol apps share. "Latest" is asked of GitHub (`releases/latest` redirects to the tag), the release's own
//! zip is downloaded, and it is checked against the `.sha256sum` file of the same release. As with yt-dlp, that checksum comes from the
//! same place as the file: it catches a damaged download, not a compromised upstream.
//!
//! Only the sites that run script in the page (YouTube) need it; SoundCloud and the others never touch it. So a missing Deno is not a
//! reason to stop the app, only a YouTube video that may not download.

use crate::{
    ffmpeg_install::{self, DownloadState},
    ytdlp::{self, Install, Latest},
};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

const REPO: &str = "https://github.com/denoland/deno";
/// The name the install gives itself in `installed.json`.
const PACK: &str = "deno";

pub fn executable_name() -> &'static str {
    if cfg!(windows) { "deno.exe" } else { "deno" }
}

/// The zip of a release that runs on this system. `None` where there is none we have tried (macOS, for the reason ffmpeg and yt-dlp have none).
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("deno-x86_64-pc-windows-msvc.zip"),
        ("windows", "aarch64") => Some("deno-aarch64-pc-windows-msvc.zip"),
        ("linux", "x86_64") => Some("deno-x86_64-unknown-linux-gnu.zip"),
        ("linux", "aarch64") => Some("deno-aarch64-unknown-linux-gnu.zip"),
        _ => None,
    }
}

/// The tag of the latest release (`v2.9.8`).
pub fn fetch_latest() -> Result<String, String> {
    ytdlp::latest_tag(REPO)
}

/// The release the install in `dir` recorded; `None` when there is none, or the program is there without a record.
pub fn installed_version(dir: &Path) -> Option<String> {
    ytdlp::recorded_version(dir, executable_name(), PACK)
}

/// The program, when an install put it in `dir`.
pub fn locate(dir: Option<&Path>) -> Option<PathBuf> {
    let dir = dir?;
    installed_version(dir)?;
    Some(dir.join(executable_name()))
}

/// What yt-dlp is told so it finds the Deno the app installed (never one on the PATH). Nothing when there is none.
pub fn runtime_args(deno: Option<&Path>) -> Vec<OsString> {
    match deno {
        Some(path) => {
            let mut spec = OsString::from("deno:");
            spec.push(path.as_os_str());
            vec![OsString::from("--js-runtimes"), spec]
        }
        None => Vec::new(),
    }
}

/// The question "which Deno is the latest?", asked in the background.
pub fn latest() -> Latest {
    Latest::of("deno latest", fetch_latest)
}

/// The install, run in the background; the window polls its `snapshot`.
pub fn installer() -> Install {
    Install::of("deno install", run)
}

/// The checksum a release's `.sha256sum` file gives: the first 64 hexadecimal digits in it. The Windows file is a PowerShell
/// `Get-FileHash` listing (`Hash : <digits>`), the others are `<digits>  <file>`; one asset per file, so there is no name to match.
fn checksum_of(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_hexdigit()).find(|w| w.len() == 64).map(str::to_ascii_lowercase)
}

fn run(dir: &Path, set: &dyn Fn(DownloadState)) -> Result<bool, String> {
    let asset = asset_name().ok_or_else(|| format!("there is no Deno for this system ({}-{}) yet", std::env::consts::OS, std::env::consts::ARCH))?;
    let tag = fetch_latest()?;
    if installed_version(dir).as_deref() == Some(tag.as_str()) {
        return Ok(false);
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let sums = {
        let mut resp = ytdlp::agent(true).get(format!("{REPO}/releases/download/{tag}/{asset}.sha256sum")).call().map_err(|e| format!("download: {e}"))?;
        resp.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|e| format!("download: {e}"))?
    };
    let expected = checksum_of(&sums).ok_or_else(|| format!("the release {tag} gives no checksum for {asset}"))?;

    let part = dir.join(format!("{asset}.part"));
    let digest = ytdlp::download_to(&format!("{REPO}/releases/download/{tag}/{asset}"), &part, set)?;
    set(DownloadState::Verifying);
    if digest != expected {
        let _ = fs::remove_file(&part);
        return Err("the download does not match the checksum of its release".to_owned());
    }
    set(DownloadState::Unpacking);
    let unpacked = ffmpeg_install::unpack_program(&part, dir, executable_name());
    let _ = fs::remove_file(&part);
    unpacked?;
    ytdlp::record(dir, PACK, &tag, &expected)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksum_is_read_from_either_kind_of_file() {
        let h = "CBA84B26E0DF5AE7F74FC66B86A95B866872A86292EBD84FA1E959927EDD93BF";
        let windows = format!("Algorithm : SHA256\r\nHash      : {h}\r\nPath      : C:\\a\\deno.zip\r\n");
        assert_eq!(checksum_of(&windows), Some(h.to_ascii_lowercase()));
        let unix = format!("{} deno-x86_64-unknown-linux-gnu.zip\n", h.to_ascii_lowercase());
        assert_eq!(checksum_of(&unix), Some(h.to_ascii_lowercase()));
        assert_eq!(checksum_of("not a checksum"), None);
    }

    #[test]
    fn yt_dlp_is_told_only_about_a_deno_the_app_installed() {
        assert!(runtime_args(None).is_empty());
        let args = runtime_args(Some(Path::new("C:/x/deno.exe")));
        assert_eq!(args, [OsString::from("--js-runtimes"), OsString::from("deno:C:/x/deno.exe")]);
    }

    /// The real thing, off by default (it downloads ~40 MB): `cargo test -p media-downloader-engine -- --ignored the_real_deno`.
    #[test]
    #[ignore]
    fn the_real_deno_installs_runs_and_is_current_afterwards() {
        let dir = std::env::temp_dir().join(format!("yaiol-mdld-deno-real-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let installer = installer();
        assert_eq!(installer.update(&dir), Ok(true));
        let exe = locate(Some(&dir)).expect("installed and recorded");
        let said = std::process::Command::new(&exe).arg("--version").output().unwrap();
        assert!(String::from_utf8_lossy(&said.stdout).starts_with("deno "), "{said:?}");
        assert_eq!(installer.update(&dir), Ok(false), "the latest is there: nothing to do");
        println!("{} {}", exe.display(), installed_version(&dir).unwrap());
    }

    #[test]
    fn a_copy_is_a_known_release_only_when_the_install_recorded_it() {
        let dir = std::env::temp_dir().join(format!("yaiol-mdld-deno-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(locate(Some(&dir)), None);
        fs::write(dir.join(executable_name()), b"x").unwrap();
        assert_eq!(locate(Some(&dir)), None, "a program put there by hand has no record");
        ytdlp::record(&dir, PACK, "v2.9.8", &"a".repeat(64)).unwrap();
        assert_eq!(installed_version(&dir).as_deref(), Some("v2.9.8"));
        assert_eq!(locate(Some(&dir)), Some(dir.join(executable_name())));
        let _ = fs::remove_dir_all(&dir);
    }
}
