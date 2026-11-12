//! The two lists of links, kept from one session to the next.
//!
//! `links.json`, beside `settings.json`: every row of the Video and the Audio list, in order, rewritten each time the
//! lists change. A row is saved with what the lookup found (so it is not asked again) and, once downloaded, with the
//! file it became. A download that was running or waiting when the app closed comes back as a link ready to download.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// The set (playlist) a link was listed in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedSet {
    pub title: String,
    pub year: String,
    pub date: String,
    pub uploader: String,
}

/// One row of a list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub url: String,
    /// The Audio list (true) or the Video list.
    pub audio: bool,
    /// The lookup answered: the fields below are known. False = the link is looked up again at the next start.
    pub looked: bool,
    pub title: String,
    pub sub: String,
    pub thumbnail: Option<String>,
    pub album: Option<String>,
    pub track: Option<(u32, u32)>,
    pub set: Option<SavedSet>,
    /// The file the download made, when it finished.
    pub done: Option<PathBuf>,
}

fn path() -> Option<PathBuf> {
    Some(crate::settings::app_dir()?.join("links.json"))
}

/// What was last read or written, so an unchanged list is not written again.
static LAST: Mutex<Option<String>> = Mutex::new(None);

pub fn load() -> Vec<Saved> {
    path().map(|p| load_from(&p)).unwrap_or_default()
}

/// Writes the lists when they differ from what the file holds.
pub fn save(rows: &[Saved]) {
    let Some(p) = path() else { return };
    let Ok(text) = serde_json::to_string_pretty(rows) else { return };
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_deref() == Some(text.as_str()) {
        return;
    }
    save_to(&p, &text);
    *last = Some(text);
}

/// A file that is missing or cannot be read is an empty list, never an error.
fn load_from(path: &Path) -> Vec<Saved> {
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    let rows: Vec<Saved> = serde_json::from_str(&text).unwrap_or_default();
    rows.into_iter().filter(|r| !r.url.is_empty()).collect()
}

fn save_to(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_come_back_as_they_were_saved() {
        let dir = std::env::temp_dir().join(format!("mdld-links-{}", std::process::id()));
        let file = dir.join("links.json");
        let rows = vec![
            Saved { url: "https://a.example/1".into(), looked: true, title: "One".into(), sub: "Someone · 3:07".into(), ..Default::default() },
            Saved {
                url: "https://a.example/2".into(),
                audio: true,
                looked: true,
                title: "Two".into(),
                album: Some("Set".into()),
                track: Some((2, 9)),
                set: Some(SavedSet { title: "Set".into(), year: "2026".into(), ..Default::default() }),
                done: Some(PathBuf::from("two.mp3")),
                ..Default::default()
            },
            Saved { url: "https://a.example/3".into(), audio: true, ..Default::default() },
        ];
        save_to(&file, &serde_json::to_string_pretty(&rows).unwrap());
        assert_eq!(load_from(&file), rows);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_or_broken_file_is_an_empty_list() {
        let dir = std::env::temp_dir().join(format!("mdld-links-bad-{}", std::process::id()));
        let file = dir.join("links.json");
        assert!(load_from(&file).is_empty());
        save_to(&file, "not json");
        assert!(load_from(&file).is_empty());
        save_to(&file, r#"[{"url":""},{"url":"https://a.example/1","unknown":1}]"#);
        assert_eq!(load_from(&file).len(), 1, "a row without a link is dropped, an unknown field is ignored");
        let _ = fs::remove_dir_all(dir);
    }
}
