//! A destination: a folder and the pattern of the file's name inside it. The user keeps a list of them in Settings and picks one in the
//! panel. The pattern is written with `{tokens}` and becomes a yt-dlp output template, so yt-dlp fills it in from what the site says.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::probe::SetInfo;

/// Id of the window-only choice that opens the system's folder picker. Not an entry of Settings; never stored.
pub const PICK_ID: &str = "pick";

/// What a pattern may name, in the order the editor offers them.
pub const TOKENS: [&str; 7] = ["title", "track", "uploader", "channel", "id", "year", "date"];

/// The pattern a new destination starts with.
pub const DEFAULT_PATTERN: &str = "{title} [{id}]";

/// What the sub-folder of a set may name: the set's title (`set`), the year and day it came out, and who published it.
pub const SET_TOKENS: [&str; 4] = ["set", "year", "date", "uploader"];

/// The sub-folder pattern a destination starts with: the set's title alone, as the app always named it.
pub const DEFAULT_SET_PATTERN: &str = "{set}";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Destination {
    pub id: String,
    pub name: String,
    /// Where the files go; empty = the system Downloads folder.
    pub folder: String,
    /// The file's name inside the folder, without the extension; `/` makes sub-folders.
    pub pattern: String,
    /// A set (a playlist, an album) goes into a sub-folder named after it, inside the folder.
    pub set_folder: bool,
    /// The name of that sub-folder, written with `{set}`, `{year}`, `{date}` and `{uploader}`.
    pub set_pattern: String,
}

impl Default for Destination {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), folder: String::new(), pattern: DEFAULT_PATTERN.to_owned(), set_folder: false, set_pattern: DEFAULT_SET_PATTERN.to_owned() }
    }
}

impl Destination {
    pub fn new(name: &str, folder: &str) -> Self {
        Self { id: crate::preset::new_id(), name: name.to_owned(), folder: folder.to_owned(), ..Self::default() }
    }

    /// An entry named after the folder it writes into.
    pub fn for_folder(dir: &Path) -> Self {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| dir.display().to_string());
        Self::new(&name, &dir.display().to_string())
    }

    /// The seeded entry of a fresh install: the system Downloads folder.
    pub fn seeded() -> Self {
        Self { id: "default-downloads".to_owned(), name: "Downloads".to_owned(), ..Self::default() }
    }

    pub fn writes_to(&self, dir: &Path) -> bool {
        !self.folder.trim().is_empty() && Path::new(self.folder.trim()) == dir
    }

    /// Where a download goes: `base`, plus a sub-folder named after its set when this destination asks for one and the item has a set.
    pub fn folder_for(&self, base: &Path, set: Option<&SetInfo>) -> PathBuf {
        match set.filter(|_| self.set_folder) {
            Some(set) => in_set(base, &set_name(&self.set_pattern, set)),
            None => base.to_path_buf(),
        }
    }

    /// The yt-dlp output template of the pattern, extension included, for a file that is `track` (its number, and how many there are)
    /// of a set; None for a file that has no number.
    pub fn template(&self, track: Option<(u32, u32)>) -> String {
        template_for(&self.pattern, track)
    }
}

/// The name a sub-folder pattern gives for `set`. A word the site did not give (a set with no date) leaves nothing, and the dashes, dots and
/// spaces it was joined by are dropped with it, so `{year}-{set}` of an undated set is just the set. An empty pattern is the title.
pub fn set_name(pattern: &str, set: &SetInfo) -> String {
    let pattern = if pattern.trim().is_empty() { DEFAULT_SET_PATTERN } else { pattern };
    let mut missing = false;
    let mut out = pattern.to_owned();
    for token in SET_TOKENS {
        let value = match token {
            "set" => set.title.as_str(),
            "year" => set.year.as_str(),
            "date" => set.date.as_str(),
            _ => set.uploader.as_str(),
        };
        let word = format!("{{{token}}}");
        if out.contains(&word) {
            missing |= value.is_empty();
            out = out.replace(&word, value);
        }
    }
    if missing { out.trim_matches(|c: char| c == '-' || c == '_' || c == '.' || c.is_whitespace()).to_owned() } else { out }
}

/// What a sub-folder pattern gives for an example set: the editor's preview.
pub fn set_preview(pattern: &str) -> String {
    set_name(pattern, &SetInfo { title: "A good album".into(), year: "2024".into(), date: "2024-05-17".into(), uploader: "Some Channel".into() })
}

/// `base` plus a folder named after `set`: what a file system cannot hold in a name is replaced, the dots and spaces at the ends are
/// dropped, and a name with nothing left means no sub-folder.
pub fn in_set(base: &Path, set: &str) -> PathBuf {
    let clean: String = set.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let clean: String = clean.trim().trim_matches('.').trim().chars().take(100).collect();
    let clean = clean.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    if clean.is_empty() { base.to_path_buf() } else { base.join(clean) }
}

/// The yt-dlp field a token stands for.
fn field(token: &str) -> Option<&'static str> {
    Some(match token {
        "title" => "%(title).150B",
        "uploader" => "%(uploader|Unknown)s",
        "channel" => "%(channel|Unknown)s",
        "id" => "%(id)s",
        "year" => "%(upload_date>%Y|Unknown)s",
        "date" => "%(upload_date>%Y-%m-%d|Unknown)s",
        _ => return None,
    })
}

/// A pattern as a yt-dlp output template. A literal `%` is doubled, `\` is a folder separator, a `..` or empty step is dropped (a pattern
/// never leaves its folder), and a name with nothing in it falls back to the default pattern.
pub fn template(pattern: &str) -> String {
    template_for(pattern, None)
}

/// The same, for a file that is `track` of a set. `{track}` is the one name yt-dlp is not asked for: each file is downloaded by itself, so
/// only the app knows its place in the set. It is written here, with as many digits as the set's last number has (two at least).
pub fn template_for(pattern: &str, track: Option<(u32, u32)>) -> String {
    let mut steps: Vec<String> = Vec::new();
    for step in pattern.replace('\\', "/").split('/') {
        let step = numbered(step.trim(), track);
        if step.is_empty() || step == "." || step == ".." || step.ends_with(':') && step.len() == 2 {
            continue;
        }
        steps.push(render_step(&step));
    }
    if steps.is_empty() {
        return template(DEFAULT_PATTERN);
    }
    format!("{}.%(ext)s", steps.join("/"))
}

/// One step of a pattern with `{track}` filled in. A file with no number leaves nothing, and the dashes, dots and spaces the number was
/// joined by are dropped with it, so `{track} - {title}` of a single video is just the title.
fn numbered(step: &str, track: Option<(u32, u32)>) -> String {
    const WORD: &str = "{track}";
    if !step.contains(WORD) {
        return step.to_owned();
    }
    match track {
        Some((n, of)) => step.replace(WORD, &format!("{n:0w$}", w = of.to_string().len().max(2))),
        None => {
            let loose = |c: char| c == '-' || c == '_' || c == '.' || c.is_whitespace();
            let parts: Vec<&str> = step.split(WORD).map(|p| p.trim_matches(loose)).filter(|p| !p.is_empty()).collect();
            parts.join(" ")
        }
    }
}

fn render_step(step: &str) -> String {
    let mut out = String::new();
    let mut rest = step;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        out.push_str(&before.replace('%', "%%"));
        match after.find('}').and_then(|close| field(&after[1..close]).map(|f| (close, f))) {
            Some((close, f)) => {
                out.push_str(f);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = &after[1..];
            }
        }
    }
    out.push_str(&rest.replace('%', "%%"));
    out
}

/// What a pattern gives for an example video, extension included: the editor's preview. Same steps as `template`.
pub fn preview(pattern: &str) -> String {
    let sample = |token: &str| match token {
        "title" => "A good song",
        "track" => "03",
        "uploader" | "channel" => "Some Channel",
        "id" => "dQw4w9WgXcQ",
        "year" => "2024",
        "date" => "2024-05-17",
        _ => "",
    };
    let mut steps: Vec<String> = Vec::new();
    for step in pattern.replace('\\', "/").split('/') {
        let step = step.trim();
        if step.is_empty() || step == "." || step == ".." || step.ends_with(':') && step.len() == 2 {
            continue;
        }
        let mut out = step.to_owned();
        for token in TOKENS {
            out = out.replace(&format!("{{{token}}}"), sample(token));
        }
        steps.push(out);
    }
    if steps.is_empty() {
        return preview(DEFAULT_PATTERN);
    }
    format!("{}.mp4", steps.join("/"))
}

/// The list as it should be kept: never empty, no repeated id, no blank name.
pub fn repaired(list: Vec<Destination>) -> Vec<Destination> {
    let mut out: Vec<Destination> = Vec::new();
    for mut d in list {
        if d.id.trim().is_empty() || out.iter().any(|x| x.id == d.id) {
            d.id = crate::preset::new_id();
        }
        if d.name.trim().is_empty() {
            d.name = d.id.clone();
        }
        if d.pattern.trim().is_empty() {
            d.pattern = DEFAULT_PATTERN.to_owned();
        }
        out.push(d);
    }
    if out.is_empty() {
        out.push(Destination::seeded());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_pattern_is_the_name_the_app_always_used() {
        assert_eq!(template(DEFAULT_PATTERN), "%(title).150B [%(id)s].%(ext)s");
    }

    #[test]
    fn tokens_become_fields_and_folders_are_kept() {
        assert_eq!(template("{uploader}/{year}/{title}"), "%(uploader|Unknown)s/%(upload_date>%Y|Unknown)s/%(title).150B.%(ext)s");
        assert_eq!(template("{uploader}\\{title}"), "%(uploader|Unknown)s/%(title).150B.%(ext)s");
    }

    #[test]
    fn a_percent_is_a_literal_and_an_unknown_token_is_left_as_typed() {
        assert_eq!(template("100% {title}"), "100%% %(title).150B.%(ext)s");
        assert_eq!(template("{nope} {id}"), "{nope} %(id)s.%(ext)s");
        // the track number is the app's: two digits at least, as many as the set's last number has
        assert_eq!(template_for("{track} - {title}", Some((3, 12))), "03 - %(title).150B.%(ext)s");
        assert_eq!(template_for("{track} {title}", Some((7, 120))), "007 %(title).150B.%(ext)s");
        assert_eq!(template_for("{track}/{title}", Some((3, 12))), "03/%(title).150B.%(ext)s");
        // a file with no number leaves nothing of it, nor of what joined it
        assert_eq!(template_for("{track} - {title}", None), "%(title).150B.%(ext)s");
        assert_eq!(template_for("{title} - {track} [{id}]", None), "%(title).150B [%(id)s].%(ext)s");
        assert_eq!(template_for("{track}/{title}", None), "%(title).150B.%(ext)s");
        assert_eq!(template_for("{track}", None), template(DEFAULT_PATTERN));
        assert_eq!(preview("{track} - {title}"), "03 - A good song.mp4");
        assert_eq!(template("a { b"), "a { b.%(ext)s");
    }

    #[test]
    fn a_pattern_never_leaves_its_folder() {
        assert_eq!(template("../../x/{id}"), "x/%(id)s.%(ext)s");
        assert_eq!(template("C:/x/{id}"), "x/%(id)s.%(ext)s");
        assert_eq!(template("/x/./{id}"), "x/%(id)s.%(ext)s");
    }

    #[test]
    fn the_preview_fills_the_tokens_the_way_the_template_does() {
        assert_eq!(preview("{uploader}/{title} [{id}]"), "Some Channel/A good song [dQw4w9WgXcQ].mp4");
        assert_eq!(preview("../x/{year}"), "x/2024.mp4");
        assert_eq!(preview(""), preview(DEFAULT_PATTERN));
    }

    #[test]
    fn an_empty_pattern_falls_back_to_the_default() {
        assert_eq!(template("  "), template(DEFAULT_PATTERN));
        assert_eq!(template("../.."), template(DEFAULT_PATTERN));
    }

    #[test]
    fn a_list_is_never_empty_and_keeps_unique_ids() {
        assert_eq!(repaired(Vec::new()).len(), 1);
        let a = Destination { id: "x".into(), name: "A".into(), ..Destination::default() };
        let list = repaired(vec![a.clone(), a]);
        assert_ne!(list[0].id, list[1].id);
    }

    #[test]
    fn a_set_gets_its_own_folder_only_when_asked() {
        let base = Path::new("/music");
        let album = SetInfo { title: "The Album".into(), ..SetInfo::default() };
        let mut d = Destination::seeded();
        assert_eq!(d.folder_for(base, Some(&album)), base);
        d.set_folder = true;
        assert_eq!(d.folder_for(base, Some(&album)), base.join("The Album"));
        assert_eq!(d.folder_for(base, None), base);
    }

    #[test]
    fn a_set_folder_pattern_may_name_the_year_and_drops_what_the_site_did_not_give() {
        let dated = SetInfo { title: "The Album".into(), year: "2017".into(), date: "2017-09-26".into(), uploader: "Band".into() };
        let undated = SetInfo { title: "The Album".into(), ..SetInfo::default() };
        assert_eq!(set_name("{year}-{set}", &dated), "2017-The Album");
        assert_eq!(set_name("{set} ({uploader}, {date})", &dated), "The Album (Band, 2017-09-26)");
        assert_eq!(set_name("{year}-{set}", &undated), "The Album");
        assert_eq!(set_name("{year} - {set}", &undated), "The Album");
        assert_eq!(set_name("  ", &dated), "The Album");
        assert_eq!(set_preview("{year}-{set}"), "2024-A good album");
        let mut d = Destination::seeded();
        d.set_folder = true;
        d.set_pattern = "{year}-{set}".into();
        assert_eq!(d.folder_for(Path::new("/m"), Some(&dated)), Path::new("/m").join("2017-The Album"));
    }

    #[test]
    fn an_older_settings_file_without_the_pattern_gets_the_title_alone() {
        let d: Destination = serde_json::from_str(r#"{"id":"x","name":"A","folder":"","pattern":"{title}","set_folder":true}"#).unwrap();
        assert_eq!(d.set_pattern, DEFAULT_SET_PATTERN);
    }

    #[test]
    fn a_set_name_is_made_safe_for_a_folder() {
        let base = Path::new("/m");
        assert_eq!(in_set(base, "AC/DC: Live?"), base.join("AC_DC_ Live_"));
        assert_eq!(in_set(base, "  ..Name.. "), base.join("Name"));
        assert_eq!(in_set(base, "..."), base);
        assert_eq!(in_set(base, "../x"), base.join("_x"));
    }

    #[test]
    fn a_folder_is_found_again_and_named_after_itself() {
        let d = Destination::for_folder(Path::new("/music/lofi"));
        assert_eq!(d.name, "lofi");
        assert!(d.writes_to(Path::new("/music/lofi")));
        assert!(!Destination::seeded().writes_to(Path::new("")));
    }
}
