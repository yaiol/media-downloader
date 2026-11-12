//! Installing and updating yt-dlp, the program that does the downloading.
//!
//! The same rule as ffmpeg (see `ffmpeg_install`): the app never uses a yt-dlp it did not install, never ships one, and keeps ONE
//! copy, the latest, in the folder the yaiol apps share. What differs is how "latest" is known. There is no manifest of ours:
//! yt-dlp must be updated within days of a site change, so the app asks GitHub which release is the latest (the address
//! `releases/latest` answers with a redirect to the tag - the API would limit an anonymous caller to 60 requests an hour),
//! downloads that release's own file, and checks it against the `SHA2-256SUMS` list of the same release. That checksum comes
//! from the same place as the file, so it catches a damaged download, not a compromised upstream; the owner accepted that
//! (PLAN-media-downloader.md, decision 4).

use crate::ffmpeg_install::DownloadState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

const REPO: &str = "https://github.com/yt-dlp/yt-dlp";

/// What an install leaves beside the program: the pack's name and the build it is, in the same shape as the ffmpeg folder's file.
const MARKER: &str = "installed.json";
const PACK: &str = "yt-dlp";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Installed {
    version: String,
    sha256: String,
}

/// Where yt-dlp keeps what it learns between runs (SoundCloud's client id, for one), beside the program. Its default folder is the user's
/// home, which may be unwritable; then every run scrapes the id again and a long set is refused part-way.
pub fn cache_dir(exe: &Path) -> std::path::PathBuf {
    exe.parent().map(|d| d.join("cache")).unwrap_or_else(|| std::path::PathBuf::from("cache"))
}

pub fn executable_name() -> &'static str {
    if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" }
}

/// The file of a release that runs on this system. `None` where there is none we have tried: macOS has no row for the same reason
/// ffmpeg has none (nobody has a Mac to test on yet).
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("yt-dlp.exe"),
        ("linux", "x86_64") => Some("yt-dlp_linux"),
        ("linux", "aarch64") => Some("yt-dlp_linux_aarch64"),
        _ => None,
    }
}

pub(crate) fn agent(follow: bool) -> ureq::Agent {
    let mut b = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(20))).timeout_recv_body(Some(Duration::from_secs(120)));
    if !follow {
        // The answer we want IS the redirect.
        b = b.max_redirects(0).max_redirects_will_error(false).http_status_as_error(false);
    }
    b.build().into()
}

/// The tag of the latest release (`2026.11.04`), read from where `releases/latest` redirects to. Blocking: a network round trip.
pub fn fetch_latest() -> Result<String, String> {
    latest_tag(REPO)
}

/// The same question for any GitHub repository (Deno asks it of its own).
pub(crate) fn latest_tag(repo: &str) -> Result<String, String> {
    let resp = agent(false).get(format!("{repo}/releases/latest")).call().map_err(|e| format!("cannot reach GitHub: {e}"))?;
    let place = resp.headers().get("location").and_then(|v| v.to_str().ok()).ok_or_else(|| format!("GitHub did not say which is the latest release of {repo}"))?;
    tag_of(place).ok_or_else(|| format!("GitHub's answer could not be read: {place}"))
}

/// Fetch `url` into the file `part`, reporting the progress, and give the SHA-256 of what was written (lower-case hex).
pub(crate) fn download_to(url: &str, part: &Path, set: &dyn Fn(DownloadState)) -> Result<String, String> {
    let mut resp = agent(true).get(url).call().map_err(|e| format!("download: {e}"))?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.body_mut().with_config().limit(1 << 29).reader();
    let mut file = fs::File::create(part).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let (mut done, mut last) = (0u64, 0u64);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if done - last >= 512 * 1024 {
            last = done;
            set(DownloadState::Downloading { done, total });
        }
    }
    Ok(hex(&hasher.finalize()))
}

/// The tag a release address ends with: `https://github.com/yt-dlp/yt-dlp/releases/tag/2026.11.04` gives `2026.11.04`.
fn tag_of(address: &str) -> Option<String> {
    let (head, tag) = address.rsplit_once("/releases/tag/")?;
    let _ = head;
    let tag = tag.split(['?', '#', '/']).next()?.trim();
    (!tag.is_empty()).then(|| tag.to_owned())
}

/// The checksum a release's `SHA2-256SUMS` gives for `asset`. Lines read `<hash>  <file>`, with a `*` before the file in binary mode.
fn checksum_of(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == asset && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_marker(dir: &Path) -> BTreeMap<String, Installed> {
    fs::read_to_string(dir.join(MARKER)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// The release the install of `pack` in `dir` recorded, when `program` is there too. `None` for a program put there by hand
/// (not a build we know).
pub(crate) fn recorded_version(dir: &Path, program: &str, pack: &str) -> Option<String> {
    dir.join(program).is_file().then_some(())?;
    read_marker(dir).remove(pack).map(|i| i.version)
}

/// The release the install in `dir` recorded. `None` when there is none, or when the program is there without a record
/// (one put there by hand is not a build we know).
pub fn installed_version(dir: &Path) -> Option<String> {
    recorded_version(dir, executable_name(), PACK)
}

/// The program, when an install put it in `dir`.
pub fn locate(dir: Option<&Path>) -> Option<PathBuf> {
    let dir = dir?;
    installed_version(dir)?;
    Some(dir.join(executable_name()))
}

/// What the app knows of the latest release, as far as this process has asked.
#[derive(Clone, Debug, PartialEq)]
pub enum LatestState {
    Unknown,
    Checking,
    Known(String),
    Failed(String),
}

/// The question "which yt-dlp is the latest?", asked in the background so a window never waits for it.
#[derive(Clone)]
pub struct Latest {
    state: Arc<Mutex<LatestState>>,
    /// What the log calls the program, and how its latest release is asked.
    label: &'static str,
    fetch: fn() -> Result<String, String>,
}

impl Default for Latest {
    fn default() -> Self {
        Latest::new()
    }
}

impl Latest {
    pub fn new() -> Latest {
        Latest::of("yt-dlp latest", fetch_latest)
    }

    /// The same question for another program installed the same way.
    pub fn of(label: &'static str, fetch: fn() -> Result<String, String>) -> Latest {
        Latest { state: Arc::new(Mutex::new(LatestState::Unknown)), label, fetch }
    }

    pub fn snapshot(&self) -> LatestState {
        self.state.lock().unwrap().clone()
    }

    /// Ask again. Does nothing while an answer is already on its way.
    pub fn check(&self) {
        {
            let mut st = self.state.lock().unwrap();
            if *st == LatestState::Checking {
                return;
            }
            *st = LatestState::Checking;
        }
        let state = Arc::clone(&self.state);
        let (label, fetch) = (self.label, self.fetch);
        std::thread::spawn(move || {
            let answer = match fetch() {
                Ok(tag) => LatestState::Known(tag),
                Err(e) => {
                    crate::log::error(label, &e);
                    LatestState::Failed(e)
                }
            };
            *state.lock().unwrap() = answer;
        });
    }
}

/// The install, run in the background; the window polls `snapshot`.
#[derive(Clone)]
pub struct Install {
    state: Arc<Mutex<DownloadState>>,
    /// What the log calls the program, and the install itself (`Ok(true)` when something was installed).
    label: &'static str,
    run: fn(&Path, &dyn Fn(DownloadState)) -> Result<bool, String>,
}

impl Default for Install {
    fn default() -> Self {
        Install::new()
    }
}

impl Install {
    pub fn new() -> Install {
        Install::of("yt-dlp install", run)
    }

    /// The same install for another program that comes the same way.
    pub fn of(label: &'static str, run: fn(&Path, &dyn Fn(DownloadState)) -> Result<bool, String>) -> Install {
        Install { state: Arc::new(Mutex::new(DownloadState::Idle)), label, run }
    }

    pub fn snapshot(&self) -> DownloadState {
        self.state.lock().unwrap().clone()
    }

    /// Install the latest yt-dlp into `dir` in the background. Does nothing while an install is already running.
    pub fn start(&self, dir: PathBuf) {
        if self.snapshot().is_active() {
            return;
        }
        let this = self.clone();
        std::thread::spawn(move || {
            let _ = this.update(&dir);
        });
    }

    /// Bring `dir` to the latest release and wait for it. `Ok(true)` when something was installed, `Ok(false)` when the latest
    /// was there already. The state is the one the window shows, so a failed download that triggers this shows "Updating yt-dlp".
    pub fn update(&self, dir: &Path) -> Result<bool, String> {
        {
            let mut st = self.state.lock().unwrap();
            if st.is_active() {
                return Err("an install is already running".to_owned());
            }
            *st = DownloadState::Downloading { done: 0, total: None };
        }
        let set = |s: DownloadState| *self.state.lock().unwrap() = s;
        match (self.run)(dir, &set) {
            Ok(changed) => {
                set(DownloadState::Done(dir.to_path_buf()));
                Ok(changed)
            }
            Err(e) => {
                crate::log::error(self.label, &format!("{e}\nfolder: {}", dir.display()));
                set(DownloadState::Failed(e.clone()));
                Err(e)
            }
        }
    }
}

fn run(dir: &Path, set: &dyn Fn(DownloadState)) -> Result<bool, String> {
    let asset = asset_name().ok_or_else(|| format!("there is no yt-dlp for this system ({}-{}) yet", std::env::consts::OS, std::env::consts::ARCH))?;
    let tag = fetch_latest()?;
    if installed_version(dir).as_deref() == Some(tag.as_str()) {
        return Ok(false);
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let sums = {
        let mut resp = agent(true).get(format!("{REPO}/releases/download/{tag}/SHA2-256SUMS")).call().map_err(|e| format!("download: {e}"))?;
        resp.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|e| format!("download: {e}"))?
    };
    let expected = checksum_of(&sums, asset).ok_or_else(|| format!("the release {tag} gives no checksum for {asset}"))?;

    let name = executable_name();
    let part = dir.join(format!("{name}.part"));
    let digest = download_to(&format!("{REPO}/releases/download/{tag}/{asset}"), &part, set)?;
    set(DownloadState::Verifying);
    if digest != expected {
        let _ = fs::remove_file(&part);
        return Err("the download does not match the checksum of its release".to_owned());
    }
    place(&part, dir)?;
    record(dir, PACK, &tag, &expected)?;
    Ok(true)
}

/// Put the checked file in place as the program. A copy already there is moved aside first, not deleted: Windows lets a running
/// program be renamed but not removed, so a yt-dlp that is downloading at this moment does not stop the replacement.
fn place(part: &Path, dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(part, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let name = executable_name();
    let target = dir.join(name);
    let aside = dir.join(format!("{name}.old"));
    let _ = fs::remove_file(&aside);
    let moved = target.exists();
    if moved {
        fs::rename(&target, &aside).map_err(|e| format!("the yt-dlp in use cannot be replaced now: {e}"))?;
    }
    if let Err(e) = fs::rename(part, &target) {
        if moved {
            let _ = fs::rename(&aside, &target);
        }
        return Err(e.to_string());
    }
    let _ = fs::remove_file(&aside);
    Ok(())
}

pub(crate) fn record(dir: &Path, pack: &str, tag: &str, sha256: &str) -> Result<(), String> {
    // The other packs of the folder keep their entries.
    let mut record = read_marker(dir);
    record.insert(pack.to_owned(), Installed { version: tag.to_owned(), sha256: sha256.to_ascii_lowercase() });
    let text = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    fs::write(dir.join(MARKER), text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yaiol-mdld-ytdlp-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_tag_is_what_the_redirect_ends_with() {
        assert_eq!(tag_of("https://github.com/yt-dlp/yt-dlp/releases/tag/2026.11.04").as_deref(), Some("2026.11.04"));
        assert_eq!(tag_of("/yt-dlp/yt-dlp/releases/tag/2026.11.04?x=1").as_deref(), Some("2026.11.04"));
        assert_eq!(tag_of("https://github.com/yt-dlp/yt-dlp/releases/tag/"), None);
        assert_eq!(tag_of("https://github.com/login"), None);
    }

    #[test]
    fn the_checksum_is_the_one_of_the_file_asked_for() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  yt-dlp\n{b} *yt-dlp.exe\n{a}  yt-dlp.exe.sig\n");
        assert_eq!(checksum_of(&sums, "yt-dlp.exe"), Some("b".repeat(64)), "binary mode and capitals");
        assert_eq!(checksum_of(&sums, "yt-dlp"), Some(a));
        assert_eq!(checksum_of(&sums, "yt-dlp_linux"), None);
        assert_eq!(checksum_of("short  yt-dlp.exe", "yt-dlp.exe"), None, "not a checksum");
    }

    #[test]
    fn a_copy_is_a_known_release_only_when_the_install_recorded_it() {
        let dir = scratch("marker");
        assert_eq!(installed_version(&dir), None);
        fs::write(dir.join(executable_name()), b"x").unwrap();
        assert_eq!(installed_version(&dir), None, "a program put there by hand has no record");
        assert_eq!(locate(Some(&dir)), None);
        fs::write(dir.join(MARKER), r#"{"ffmpeg":{"version":"9.0.2","sha256":"ab"}}"#).unwrap();
        assert_eq!(installed_version(&dir), None, "another pack's record is not yt-dlp's");
        record(&dir, PACK, "2026.11.04", &"c".repeat(64)).unwrap();
        assert_eq!(installed_version(&dir).as_deref(), Some("2026.11.04"));
        assert_eq!(locate(Some(&dir)), Some(dir.join(executable_name())));
        assert!(read_marker(&dir).contains_key("ffmpeg"), "the other pack keeps its entry");
        fs::remove_file(dir.join(executable_name())).unwrap();
        assert_eq!(installed_version(&dir), None, "a record without the program is nothing");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_program_replaces_an_older_one_and_leaves_nothing_behind() {
        let dir = scratch("place");
        let name = executable_name();
        fs::write(dir.join(name), b"the old one").unwrap();
        let part = dir.join(format!("{name}.part"));
        fs::write(&part, b"the new one").unwrap();
        place(&part, &dir).unwrap();
        assert_eq!(fs::read(dir.join(name)).unwrap(), b"the new one");
        assert!(!part.exists());
        assert!(!dir.join(format!("{name}.old")).exists());
        let _ = fs::remove_dir_all(dir);
    }

    /// Needs the network: the real GitHub answers with a tag that looks like a date. Run with `--ignored`.
    #[test]
    #[ignore]
    fn github_names_the_latest_release() {
        let tag = fetch_latest().unwrap();
        assert!(tag.chars().next().unwrap().is_ascii_digit(), "{tag}");
        assert!(tag.contains('.'), "{tag}");
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Needs the network: installs the real latest yt-dlp into a scratch folder, runs it, and looks a link up with it. Run with `--ignored`.
    #[test]
    #[ignore]
    fn the_latest_yt_dlp_installs_and_runs_and_looks_a_link_up() {
        let dir = std::env::temp_dir().join(format!("yaiol-mdld-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let install = Install::new();
        assert_eq!(install.update(&dir), Ok(true));
        let tag = fetch_latest().unwrap();
        assert_eq!(installed_version(&dir).as_deref(), Some(tag.as_str()));
        assert_eq!(install.update(&dir), Ok(false), "the second time there is nothing to do");
        let exe = locate(Some(&dir)).unwrap();
        let version = crate::ffmpeg::command(&exe).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), tag);
        let items = crate::probe::probe(&exe, None, "https://www.youtube.com/watch?v=jNQXAC9IVRw", None, None).unwrap();
        assert_eq!(items.len(), 1);
        eprintln!("{items:?}");
        let _ = fs::remove_dir_all(dir);
    }
}
