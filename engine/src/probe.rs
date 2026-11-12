//! Looking a link up: what is behind it, before anything is downloaded. A link to one video gives one item; a link to a playlist
//! gives one item per video (the owner's call: the list is the one place where every download is seen).

use crate::browsers::Signin;
use serde_json::Value;
use std::{
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
};

/// How far a lookup has got, readable from the window while it runs: the listing itself has no end in sight (total 0), then each track
/// that came without a title is looked up (`done` of `total`).
#[derive(Clone, Debug, Default)]
pub struct Progress(Arc<(AtomicU32, AtomicU32, AtomicBool)>);

impl Progress {
    /// The lookup is under way. Until then it waits for its turn (another link is being looked up, or a download is running).
    pub fn started(&self) -> bool {
        self.0 .2.load(Ordering::Relaxed)
    }
    pub(crate) fn start(&self) {
        self.0 .2.store(true, Ordering::Relaxed);
    }
    /// A share between 0 and 1, or `None` while it cannot be told.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.0 .1.load(Ordering::Relaxed);
        (total > 0).then(|| (self.0 .0.load(Ordering::Relaxed) as f32 / total as f32).min(1.0))
    }
    fn plan(&self, total: usize) {
        self.0 .1.store(total as u32, Ordering::Relaxed);
    }
    fn step(&self) {
        self.0 .0.fetch_add(1, Ordering::Relaxed);
    }
}

impl PartialEq for Progress {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The address of this one video, which is what gets downloaded.
    pub url: String,
    pub title: String,
    pub duration: Option<u64>,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
    /// The source holds sound only (a music site, or every format it lists has no picture): the audio presets are the ones that make sense.
    pub audio_only: bool,
    /// The album this track belongs to and its place in it (`track`, of `tracks`): only for a track of a set on a sound site, where the set IS the album.
    pub album: Option<String>,
    pub track: Option<(u32, u32)>,
    /// The set (playlist) this item was listed in, on any site: what a destination may name a sub-folder after.
    pub set: Option<SetInfo>,
    /// The listing gave a real title. A track of a SoundCloud set comes with an id and an address only, so `title` is its id until it is looked up.
    pub titled: bool,
}

/// What the listing says about a set: the words a destination's sub-folder pattern may use.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SetInfo {
    pub title: String,
    /// The year the set came out, else the year it was uploaded; empty when the site says neither.
    pub year: String,
    /// The same day as `YYYY-MM-DD`.
    pub date: String,
    pub uploader: String,
}

impl SetInfo {
    fn read(v: &Value) -> Option<Self> {
        let title = v.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty())?.to_owned();
        let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
        // yt-dlp writes a day as `YYYYMMDD`: the release day first, because an album's year is the one it came out.
        let day = ["release_date", "upload_date", "modified_date"].iter().map(|k| text(k)).find(|d| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit())).unwrap_or_default();
        let (year, date) = if day.is_empty() { (String::new(), String::new()) } else { (day[..4].to_owned(), format!("{}-{}-{}", &day[..4], &day[4..6], &day[6..])) };
        let uploader = [text("uploader"), text("channel")].into_iter().find(|u| !u.is_empty()).unwrap_or_default();
        Some(Self { title, year, date, uploader })
    }
}

/// Ask yt-dlp what `url` is, as a visitor signed in with `signin` when one is given. Blocking: one network round trip, a little
/// longer for a long playlist.
pub fn probe(ytdlp: &Path, deno: Option<&Path>, url: &str, signin: Option<&Signin>, progress: Option<&Progress>) -> Result<Vec<Item>, String> {
    let mut command = crate::ffmpeg::command(ytdlp);
    command.args(["--ignore-config", "--no-warnings", "-J", "--flat-playlist", "--cache-dir"]);
    command.arg(crate::ytdlp::cache_dir(ytdlp));
    command.args(crate::deno::runtime_args(deno));
    if let Some(s) = signin {
        command.args(s.args());
    }
    let out = command
        .args(["--", url])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("yt-dlp could not start: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let line = text.lines().rev().find(|l| l.contains("ERROR")).or_else(|| text.lines().last()).unwrap_or("yt-dlp did not understand this link");
        return Err(line.trim().trim_start_matches("ERROR:").trim().to_owned());
    }
    let mut items = parse(&String::from_utf8_lossy(&out.stdout), url)?;
    name_untitled(ytdlp, deno, &mut items, signin, progress);
    Ok(items)
}

/// A playlist listing may carry no title for its tracks (a SoundCloud set: an id and an address). Look each of those up by itself, a few at a time;
/// a track that cannot be looked up keeps its id as its name (the download still works and yt-dlp names the file).
fn name_untitled(ytdlp: &Path, deno: Option<&Path>, items: &mut [Item], signin: Option<&Signin>, progress: Option<&Progress>) {
    let todo: Vec<usize> = (0..items.len()).filter(|n| !items[*n].titled).collect();
    if let Some(p) = progress {
        p.plan(todo.len());
    }
    for batch in todo.chunks(4) {
        let found: Vec<(usize, Option<Item>)> = std::thread::scope(|scope| {
            let jobs: Vec<_> = batch
                .iter()
                .map(|n| {
                    let url = items[*n].url.clone();
                    (*n, scope.spawn(move || probe(ytdlp, deno, &url, signin, None).ok().and_then(|v| v.into_iter().next())))
                })
                .collect();
            jobs.into_iter().map(|(n, j)| (n, j.join().ok().flatten())).collect()
        });
        for (n, one) in found {
            if let Some(p) = progress {
                p.step();
            }
            if let Some(one) = one.filter(|o| o.titled) {
                let i = &mut items[n];
                // The listing gave an api address (a SoundCloud stub); the lookup knows the track's own page, which is what the file's
                // comment should carry and what the download is better asked for.
                i.url = one.url;
                (i.title, i.titled) = (one.title, true);
                i.duration = i.duration.or(one.duration);
                i.uploader = i.uploader.take().or(one.uploader);
                i.thumbnail = i.thumbnail.take().or(one.thumbnail);
            }
        }
    }
}

/// Read yt-dlp's `-J` answer. `asked` is the address that was looked up, used when the answer does not name its own.
pub fn parse(json: &str, asked: &str) -> Result<Vec<Item>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("yt-dlp's answer could not be read: {e}"))?;
    let Some(entries) = v.get("entries").and_then(Value::as_array) else {
        return Ok(vec![item(&v, Some(asked), None).ok_or_else(|| "this link gives nothing to download".to_owned())?]);
    };
    let site = v.get("extractor_key").and_then(Value::as_str);
    let mut items: Vec<Item> = entries.iter().filter_map(|e| item(e, None, site)).collect();
    // A set on a sound site is an album: its title, and each track's place in it (a playlist of videos is not an album).
    let album = v.get("title").and_then(Value::as_str).filter(|t| !t.is_empty());
    if let (Some(album), true) = (album, site.is_some_and(|s| is_sound_site(&s.to_lowercase()))) {
        let total = items.len() as u32;
        for (n, i) in items.iter_mut().enumerate() {
            i.album = Some(album.to_owned());
            i.track = Some((n as u32 + 1, total));
        }
    }
    if let Some(set) = SetInfo::read(&v) {
        for i in items.iter_mut() {
            i.set = Some(set.clone());
        }
    }
    if items.is_empty() { Err("this playlist has nothing to download".to_owned()) } else { Ok(items) }
}

/// Sites that only ever hold sound; a playlist's entries do not list their formats, so the site is what tells.
const SOUND_SITES: [&str; 6] = ["soundcloud", "bandcamp", "mixcloud", "audiomack", "audioboom", "hearthis"];

fn is_sound_site(key: &str) -> bool {
    SOUND_SITES.iter().any(|s| key.starts_with(s))
}

fn item(v: &Value, asked: Option<&str>, site: Option<&str>) -> Option<Item> {
    let text = |key: &str| v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    let id = text("id");
    let url = text("webpage_url").or_else(|| text("url")).or_else(|| match (text("ie_key").as_deref(), &id) {
        (Some("Youtube"), Some(id)) => Some(format!("https://www.youtube.com/watch?v={id}")),
        _ => None,
    });
    // A single video names its own address; otherwise the address that was asked is the best there is. An entry of a playlist has no such fallback.
    let url = url.or_else(|| asked.map(str::to_owned))?;
    let titled = text("title").is_some();
    let title = text("title").or(id).unwrap_or_else(|| url.clone());
    let duration = v.get("duration").and_then(Value::as_f64).filter(|d| *d >= 0.0).map(|d| d.round() as u64);
    let thumbnail = text("thumbnail").or_else(|| {
        v.get("thumbnails").and_then(Value::as_array).and_then(|t| t.last()).and_then(|t| t.get("url")).and_then(Value::as_str).map(str::to_owned)
    });
    let formats = v.get("formats").and_then(Value::as_array).filter(|f| !f.is_empty());
    let no_picture = |f: &Value| f.get("vcodec").and_then(Value::as_str) == Some("none");
    let key = text("ie_key").or_else(|| text("extractor_key")).or_else(|| site.map(str::to_owned)).unwrap_or_default().to_lowercase();
    let audio_only = match formats {
        Some(f) => f.iter().all(no_picture),
        None => is_sound_site(&key),
    };
    Some(Item { url, title, duration, uploader: text("uploader").or_else(|| text("channel")), thumbnail, audio_only, album: None, track: None, set: None, titled })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_video_is_one_item() {
        let json = r#"{"id":"abc","title":"A film","webpage_url":"https://example.test/watch?v=abc","duration":61.4,"uploader":"Someone","thumbnail":"https://example.test/t.jpg"}"#;
        let items = parse(json, "https://short.test/abc").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://example.test/watch?v=abc");
        assert_eq!((items[0].title.as_str(), items[0].duration), ("A film", Some(61)));
        assert_eq!(items[0].uploader.as_deref(), Some("Someone"));
    }

    #[test]
    fn a_source_with_no_picture_is_audio_only() {
        let sound = r#"{"id":"a","title":"T","webpage_url":"https://example.test/a","formats":[{"vcodec":"none"},{"vcodec":"none"}]}"#;
        assert!(parse(sound, "x").unwrap()[0].audio_only);
        let film = r#"{"id":"a","title":"T","webpage_url":"https://example.test/a","formats":[{"vcodec":"none"},{"vcodec":"avc1"}]}"#;
        assert!(!parse(film, "x").unwrap()[0].audio_only);
        let set = r#"{"_type":"playlist","extractor_key":"SoundcloudSet","entries":[{"id":"a","title":"One","url":"https://example.test/a"}]}"#;
        assert!(parse(set, "x").unwrap()[0].audio_only);
        let tube = r#"{"_type":"playlist","extractor_key":"YoutubeTab","entries":[{"id":"a","title":"One","url":"https://example.test/a"}]}"#;
        assert!(!parse(tube, "x").unwrap()[0].audio_only);
    }

    #[test]
    fn a_set_on_a_sound_site_is_an_album_and_a_playlist_of_videos_is_not() {
        let entries = r#""entries":[{"id":"a","title":"One","url":"https://example.test/a"},null,{"id":"b","title":"Two","url":"https://example.test/b"}]"#;
        let set = parse(&format!(r#"{{"_type":"playlist","title":"The Album","extractor_key":"SoundcloudSet",{entries}}}"#), "x").unwrap();
        assert_eq!((set[0].album.as_deref(), set[0].track), (Some("The Album"), Some((1, 2))));
        assert_eq!(set[1].track, Some((2, 2)), "the numbering follows the tracks that are kept");
        let tube = parse(&format!(r#"{{"_type":"playlist","title":"Mix","extractor_key":"YoutubeTab",{entries}}}"#), "x").unwrap();
        assert_eq!((tube[0].album.clone(), tube[0].track), (None, None));
        // Both are sets, whatever the site: the title a destination may name a sub-folder after.
        let title = |i: &Item| i.set.as_ref().map(|s| s.title.clone());
        assert_eq!((title(&set[0]), title(&tube[0])), (Some("The Album".to_owned()), Some("Mix".to_owned())));
    }

    #[test]
    fn a_set_carries_the_year_it_came_out_else_the_year_it_was_uploaded() {
        let entries = r#""entries":[{"id":"a","title":"One","url":"https://example.test/a"}]"#;
        let json = |dates: &str| format!(r#"{{"_type":"playlist","title":"The Album","uploader":"Band",{dates}{entries}}}"#);
        let both = parse(&json(r#""release_date":"20170926","upload_date":"20170922","#), "x").unwrap();
        assert_eq!(both[0].set, Some(SetInfo { title: "The Album".into(), year: "2017".into(), date: "2017-09-26".into(), uploader: "Band".into() }));
        let up = parse(&json(r#""upload_date":"20121011","#), "x").unwrap();
        assert_eq!((up[0].set.as_ref().unwrap().year.as_str(), up[0].set.as_ref().unwrap().date.as_str()), ("2012", "2012-10-11"));
        let none = parse(&json(""), "x").unwrap();
        assert_eq!(none[0].set.as_ref().unwrap().year, "");
    }

    #[test]
    fn a_playlist_is_one_item_per_video_and_unusable_entries_are_skipped() {
        let json = r#"{"_type":"playlist","title":"List","entries":[
            {"id":"a","title":"One","url":"https://example.test/a","duration":10},
            null,
            {"id":"b","ie_key":"Youtube","title":"Two"},
            {"title":"No address at all"}
        ]}"#;
        let items = parse(json, "https://example.test/list").unwrap();
        let urls: Vec<&str> = items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(urls, ["https://example.test/a", "https://www.youtube.com/watch?v=b"]);
    }

    #[test]
    fn nothing_to_download_is_said_and_garbage_does_not_panic() {
        assert!(parse(r#"{"_type":"playlist","entries":[]}"#, "x").is_err());
        assert!(parse("<html>", "x").is_err());
        assert!(parse("", "x").is_err());
    }
}
