//! Persisted user settings: language and theme, in `settings.json` under the OS app-data folder.
//!
//! The folder mirrors the electron convention exactly: `…\AppData\Roaming\yaiol\<Product Name>`,
//! with a ` (Dev)` suffix on a debug build so a dev run and an installed copy never share state.
//! The file name is plain (`settings.json`) — the folder is already per-app, so there is no
//! storage-prefix to namespace it.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lang: String,
    pub theme: String,
    /// The video presets, in the order the user keeps them: a fresh install holds the seeded ones, then the list is the user's.
    pub presets: Vec<engine::preset::Preset>,
    /// The video preset picked in the panel (an id of `presets`).
    pub preset_id: String,
    /// The audio presets, kept apart from the video ones (Settings > Audio), used for a link that holds sound only.
    pub audios: Vec<engine::preset::Preset>,
    /// The audio preset picked in the panel (an id of `audios`).
    pub audio_id: String,
    /// The destinations (a folder and a file-name pattern each), in the order the user keeps them.
    pub destinations: Vec<engine::destination::Destination>,
    /// The destination picked in the panel for the video list (an id of `destinations`).
    pub destination_id: String,
    /// The destination picked in the panel for the audio list: its own pick, from the same `destinations`.
    pub audio_destination_id: String,
    /// A file that is already there is replaced (otherwise it is left alone).
    pub replace: bool,
    /// How many download at once.
    pub parallel: u32,
    /// When a video needs a signed-in visitor, borrow the login of a browser on this PC (see `engine::browsers`).
    pub login: bool,
    /// A cookies.txt the user exported from a signed-in browser; empty = none. Tried before the browsers (see `engine::browsers::Signin`).
    pub cookies_file: String,
    /// Write the title, artist, album, track number and cover into an audio file.
    pub tag: bool,
    /// A folder picked with "Pick Destination…": used for this run only, never added to `destinations` (so it is not saved).
    #[serde(skip)]
    pub picked: Option<engine::destination::Destination>,
    /// The same one-off for the audio list.
    #[serde(skip)]
    pub audio_picked: Option<engine::destination::Destination>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { lang: "en".to_owned(), theme: "light".to_owned(), presets: engine::preset::seeded(false), preset_id: "video-best".to_owned(), audios: engine::preset::seeded(true), audio_id: "audio-mp3".to_owned(), destinations: vec![engine::destination::Destination::seeded()], destination_id: "default-downloads".to_owned(), audio_destination_id: "default-downloads".to_owned(), replace: false, parallel: 2, login: true, cookies_file: String::new(), tag: true, picked: None, audio_picked: None }
    }
}

impl Settings {
    /// The id of the destination picked for a kind (the video list or the audio list).
    pub fn destination_id_of(&self, audio: bool) -> &str {
        if audio { &self.audio_destination_id } else { &self.destination_id }
    }

    pub fn set_destination(&mut self, audio: bool, id: String) {
        if audio { self.audio_destination_id = id } else { self.destination_id = id }
    }

    /// The one-off folder of a kind, if one was picked.
    pub fn one_off(&self, audio: bool) -> Option<&engine::destination::Destination> {
        if audio { self.audio_picked.as_ref() } else { self.picked.as_ref() }
    }

    /// The destination picked for a kind, or the first one when the saved id is no longer in the list.
    pub fn destination(&self, audio: bool) -> engine::destination::Destination {
        let id = self.destination_id_of(audio);
        if let Some(one) = self.one_off(audio).filter(|d| d.id == id) {
            return one.clone();
        }
        let list = engine::destination::repaired(self.destinations.clone());
        list.iter().find(|d| d.id == id).cloned().unwrap_or_else(|| list[0].clone())
    }

    /// The id of the destination that writes into `dir`: one already saved for it, or a new entry named after the folder.
    pub fn destination_for_folder(&mut self, dir: &Path) -> String {
        if let Some(d) = self.destinations.iter().find(|d| d.writes_to(dir)) {
            return d.id.clone();
        }
        let d = engine::destination::Destination::for_folder(dir);
        let id = d.id.clone();
        self.destinations.push(d);
        id
    }

    /// Use `dir` for the downloads of a kind without saving a destination for it: a saved one that already writes there is chosen, else a one-off.
    pub fn pick_folder_once(&mut self, audio: bool, dir: &Path) {
        if let Some(id) = self.destinations.iter().find(|d| d.writes_to(dir)).map(|d| d.id.clone()) {
            self.set_destination(audio, id);
            return;
        }
        let mut one = engine::destination::Destination::for_folder(dir);
        one.id = if audio { "picked-once-audio" } else { "picked-once" }.to_owned();
        self.set_destination(audio, one.id.clone());
        if audio { self.audio_picked = Some(one) } else { self.picked = Some(one) }
    }

    /// The list of presets of a kind.
    pub fn list(&self, audio: bool) -> &Vec<engine::preset::Preset> {
        if audio { &self.audios } else { &self.presets }
    }

    pub fn list_mut(&mut self, audio: bool) -> &mut Vec<engine::preset::Preset> {
        if audio { &mut self.audios } else { &mut self.presets }
    }

    /// The id picked in the panel for a kind.
    pub fn picked_id(&self, audio: bool) -> &str {
        if audio { &self.audio_id } else { &self.preset_id }
    }

    pub fn set_picked(&mut self, audio: bool, id: String) {
        if audio { self.audio_id = id } else { self.preset_id = id }
    }

    /// The preset picked for a kind, or the first one of that list when the saved id is no longer in it.
    pub fn preset_for(&self, audio: bool) -> engine::preset::Preset {
        let list = engine::preset::repaired(self.list(audio).clone(), audio);
        list.iter().find(|p| p.id == self.picked_id(audio)).cloned().unwrap_or_else(|| list[0].clone())
    }
}

/// The folder a destination writes into: the one it names, else the system Downloads folder.
pub fn folder_of(destination: &engine::destination::Destination) -> PathBuf {
    let chosen = PathBuf::from(destination.folder.trim());
    if !destination.folder.trim().is_empty() && chosen.is_absolute() {
        return chosen;
    }
    dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

/// The folder every yaiol app shares for the programs it installs (per user, so no administrator rights).
fn shared_bin() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("yaiol").join("Common Files").join("bin"))
}

/// ffmpeg: one subfolder per manifest schema, so an older app never meets a pair it cannot read.
pub fn ffmpeg_dir() -> Option<PathBuf> {
    Some(shared_bin()?.join(engine::ffmpeg_install::SCHEMA.to_string()))
}

/// yt-dlp: ONE copy, the latest, in a folder of its own (it has no schema; see PLAN-media-downloader.md).
pub fn ytdlp_dir() -> Option<PathBuf> {
    Some(shared_bin()?.join("yt-dlp"))
}

/// Deno (the script runtime YouTube needs): ONE copy, the latest, in a folder of its own, installed like yt-dlp.
pub fn deno_dir() -> Option<PathBuf> {
    Some(shared_bin()?.join("deno"))
}

/// The most downloads that run at once. A download is bound by the network, not the processor, so this is a courtesy to the sites.
pub const MAX_PARALLEL: u32 = 6;

/// The app's own folder in the user's app data (a debug build has its own, so a dev run and an installed copy never share state).
pub fn app_dir() -> Option<PathBuf> {
    let folder = if cfg!(debug_assertions) {
        format!("{} (Dev)", crate::APP_NAME)
    } else {
        crate::APP_NAME.to_owned()
    };
    Some(dirs::data_dir()?.join("yaiol").join(folder))
}

fn path() -> Option<PathBuf> {
    Some(app_dir()?.join("settings.json"))
}

/// Where the log file is kept: what to send when something goes wrong. Never sent anywhere by the app.
pub fn log_dir() -> Option<PathBuf> {
    Some(app_dir()?.join("logs"))
}

pub fn load() -> Settings {
    path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn save(settings: &Settings) {
    if let Some(p) = path() {
        save_to(&p, settings);
    }
}

fn load_from(path: &Path) -> Settings {
    let Ok(text) = fs::read_to_string(path) else { return Settings::default() };
    let Ok(mut s) = serde_json::from_str::<Settings>(&text) else { return Settings::default() };
    // A file from before the presets were two lists held both kinds in one: they are told apart by what each one is.
    let raw = serde_json::from_str::<serde_json::Value>(&text).ok();
    if let Some(old) = raw.as_ref().and_then(|v| v.get("presets")).and_then(|p| p.as_array()).filter(|_| !text.contains("\"audios\"")) {
        let all: Vec<engine::preset::Preset> = old.iter().filter_map(|p| serde_json::from_value(p.clone()).ok()).collect();
        s.audios = all.iter().filter(|p| p.is_audio()).cloned().collect();
        if let Some(picked) = all.iter().find(|p| p.id == s.preset_id && p.is_audio()) {
            s.audio_id = picked.id.clone();
        }
    }
    s.presets = engine::preset::repaired(std::mem::take(&mut s.presets), false);
    s.audios = engine::preset::repaired(std::mem::take(&mut s.audios), true);
    // A file from before destinations held ONE folder: it becomes a destination of its own, and the picked one.
    if let Some(old) = raw.as_ref().and_then(|v| v.get("folder").and_then(|f| f.as_str().map(str::to_owned))) {
        if !old.trim().is_empty() && !text.contains("\"destinations\"") {
            let id = s.destination_for_folder(Path::new(old.trim()));
            s.destination_id = id;
        }
    }
    // A file from before the audio list had a pick of its own: it starts on the one the file had.
    if !text.contains("\"audio_destination_id\"") {
        s.audio_destination_id = s.destination_id.clone();
    }
    s.destinations = engine::destination::repaired(std::mem::take(&mut s.destinations));
    s
}

fn save_to(path: &Path, settings: &Settings) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("yaiol-settings-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn round_trips() {
        let p = temp("roundtrip").join("settings.json");
        let s = Settings { lang: "ar".to_owned(), theme: "dark".to_owned(), presets: engine::preset::seeded(false), preset_id: "video-mp4-720".to_owned(), audios: engine::preset::seeded(true), audio_id: "audio-opus".to_owned(), destinations: vec![engine::destination::Destination::new("Music", r"C:/music")], destination_id: "x".to_owned(), audio_destination_id: "y".to_owned(), replace: true, parallel: 4, login: false, cookies_file: "C:/x/cookies.txt".to_owned(), tag: false, picked: None, audio_picked: None };
        save_to(&p, &s);
        assert!(load_from(&p) == s);
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn each_kind_has_its_own_list_and_its_own_pick() {
        let s = Settings { preset_id: "video-mp4-720".to_owned(), audio_id: "audio-m4a".to_owned(), ..Settings::default() };
        assert_eq!(s.preset_for(true).id, "audio-m4a");
        assert_eq!(s.preset_for(false).id, "video-mp4-720");
        let s = Settings { preset_id: "gone".to_owned(), audio_id: "video-mp4-720".to_owned(), ..Settings::default() };
        assert_eq!(s.preset_for(false).id, "video-best");
        assert_eq!(s.preset_for(true).id, "audio-best", "an id that is not in the audio list is ignored");
    }

    #[test]
    fn an_older_file_without_lists_gets_the_seeded_ones_and_an_edited_list_is_kept() {
        let p = temp("presets");
        fs::write(&p, r#"{ "preset_id": "video-mp4-720" }"#).unwrap();
        let s = load_from(&p);
        assert_eq!((s.presets, s.audios), (engine::preset::seeded(false), engine::preset::seeded(true)));
        let mut s = Settings::default();
        s.presets.truncate(2);
        s.presets[0].name = "Mine".to_owned();
        s.audios.truncate(1);
        save_to(&p, &s);
        let back = load_from(&p);
        assert_eq!((back.presets.len(), back.audios.len()), (2, 1));
        assert_eq!(back.presets[0].name, "Mine");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn an_older_file_with_one_mixed_list_is_split_by_kind() {
        let p = temp("mixed");
        let mixed = engine::preset::seeded(false).into_iter().take(1).chain(engine::preset::seeded(true).into_iter().filter(|p| p.id == "audio-m4a")).collect::<Vec<_>>();
        fs::write(&p, format!(r#"{{ "presets": {}, "preset_id": "audio-m4a" }}"#, serde_json::to_string(&mixed).unwrap())).unwrap();
        let s = load_from(&p);
        assert_eq!(s.presets.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["video-best"]);
        assert_eq!(s.audios.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["audio-m4a"]);
        assert_eq!(s.audio_id, "audio-m4a", "the audio preset that was picked stays picked");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn a_relative_or_empty_folder_means_the_downloads_folder() {
        let d = engine::destination::Destination::new("Mine", "music");
        assert!(folder_of(&d).is_absolute());
        assert!(folder_of(&Settings::default().destination(false)).is_absolute());
    }

    #[test]
    fn the_single_folder_of_an_older_file_becomes_the_picked_destination() {
        let p = temp("old-folder");
        fs::write(&p, r#"{ "folder": "C:/music/lofi" }"#).unwrap();
        let s = load_from(&p);
        assert_eq!(s.destination(false).name, "lofi");
        assert_eq!(s.destination(false).folder, r"C:/music/lofi");
        assert_eq!(s.destination(true).name, "lofi", "the audio list starts on the folder the file had");
        assert_eq!(s.destinations.len(), 2);
        let _ = fs::remove_file(p);
    }

    #[test]
    fn a_destination_that_is_no_longer_in_the_list_falls_back_to_the_first() {
        let s = Settings { destination_id: "gone".to_owned(), ..Settings::default() };
        assert_eq!(s.destination(false).id, "default-downloads");
    }

    #[test]
    fn the_video_list_and_the_audio_list_each_keep_their_own_destination() {
        let mut s = Settings::default();
        s.destinations.push(engine::destination::Destination::new("Music", r"C:/music"));
        let music = s.destinations[1].id.clone();
        s.set_destination(true, music.clone());
        assert_eq!((s.destination(false).id.as_str(), s.destination(true).id), ("default-downloads", music.clone()));
        // A one-off folder belongs to the list it was picked for.
        s.pick_folder_once(false, Path::new(r"C:/films/once"));
        assert_eq!(s.destination(false).id, "picked-once");
        assert_eq!(s.destination(true).id, music);
        assert!(s.one_off(true).is_none());
        // A folder a saved destination already writes to picks that one, for the list asked.
        s.pick_folder_once(false, Path::new(r"C:/music"));
        assert_eq!(s.destination(false).id, music);
        let p = temp("two-picks").join("settings.json");
        save_to(&p, &s);
        let back = load_from(&p);
        assert_eq!((back.destination(false).id, back.destination(true).id), (music.clone(), music));
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn missing_or_broken_file_gives_defaults() {
        assert!(load_from(&temp("missing")) == Settings::default());
        let p = temp("broken");
        fs::write(&p, "{ not json").unwrap();
        assert!(load_from(&p) == Settings::default());
        let _ = fs::remove_file(p);
    }

    #[test]
    fn a_partial_file_keeps_the_rest_at_defaults() {
        let p = temp("partial");
        fs::write(&p, r#"{ "theme": "dark" }"#).unwrap();
        let s = load_from(&p);
        assert_eq!((s.lang.as_str(), s.theme.as_str()), ("en", "dark"));
        assert_eq!((s.preset_id.as_str(), s.parallel), ("video-best", 2));
        let _ = fs::remove_file(p);
    }
}
