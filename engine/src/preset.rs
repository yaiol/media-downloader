//! A preset: what to fetch and in what form. Picked once for the whole run in the window's panel (a link carries no choice of its
//! own), like the converters' presets. A fresh install is seeded with `seeded()`; the list is then the user's, saved in
//! `settings.json` and edited on the Settings page.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Container {
    /// No file type asked for: yt-dlp's own best picture and best sound, in the file type that holds them.
    Best,
    Mp4,
    Mkv,
    Webm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    /// The sound as the site has it: taken out of its file, never converted (so the file type is the site's).
    Best,
    Mp3,
    M4a,
    Opus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    /// The picture and the sound, the picture no taller than `max_height` when that is set.
    Video { max_height: Option<u32>, container: Container },
    /// The sound alone, converted to `format`.
    Audio { format: AudioFormat },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub mode: Mode,
}

impl Preset {
    fn video(id: &str, name: &str, max_height: Option<u32>, container: Container) -> Preset {
        Preset { id: id.to_owned(), name: name.to_owned(), mode: Mode::Video { max_height, container } }
    }

    fn audio(id: &str, name: &str, format: AudioFormat) -> Preset {
        Preset { id: id.to_owned(), name: name.to_owned(), mode: Mode::Audio { format } }
    }

    /// A new preset the user is about to edit: the best video in an MP4, or a sound in MP3 (the forms that play everywhere).
    pub fn new(name: &str, audio: bool) -> Preset {
        let mode = if audio { Mode::Audio { format: AudioFormat::Mp3 } } else { Mode::Video { max_height: None, container: Container::Mp4 } };
        Preset { id: new_id(), name: name.to_owned(), mode }
    }

    pub fn is_audio(&self) -> bool {
        matches!(self.mode, Mode::Audio { .. })
    }

    /// The arguments that make yt-dlp fetch this form. yt-dlp chooses among the formats a site offers by a sort order, not a
    /// fixed list, so each preset only says what to prefer: the resolution (up to the limit), then the codecs the container holds.
    pub fn args(&self) -> Vec<String> {
        let s = |v: &str| v.to_owned();
        match &self.mode {
            Mode::Video { max_height, container } => {
                let res = match max_height {
                    Some(h) => format!("res:{h}"),
                    None => s("res"),
                };
                let (sort, merge) = match container {
                    // Nothing but the height: yt-dlp's own order decides the rest, and it merges into whatever holds the pair.
                    Container::Best => return vec![s("-S"), res],
                    Container::Mp4 => (format!("{res},vcodec:h264,acodec:m4a"), "mp4"),
                    Container::Webm => (format!("{res},vcodec:vp9,acodec:opus"), "webm"),
                    Container::Mkv => (res, "mkv"),
                };
                vec![s("-S"), sort, s("--merge-output-format"), s(merge)]
            }
            Mode::Audio { format } => {
                vec![s("-f"), s("ba/b"), s("-x"), s("--audio-format"), s(format.id()), s("--audio-quality"), s("0")]
            }
        }
    }
}

/// The heights the editor offers (a site rarely has more); a saved preset may hold any other.
pub const HEIGHTS: [u32; 6] = [2160, 1440, 1080, 720, 480, 360];

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique id without a dependency: wall-clock nanoseconds plus a process counter.
pub fn new_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{nanos:x}-{:x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

impl Container {
    pub const ALL: [Container; 4] = [Container::Best, Container::Mp4, Container::Mkv, Container::Webm];

    pub fn id(self) -> &'static str {
        match self {
            Container::Best => "best",
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
        }
    }

    pub fn from_id(id: &str) -> Option<Container> {
        Container::ALL.into_iter().find(|c| c.id() == id)
    }

    /// The name shown in a list: a file type, the same in every language. Empty for `Best`, which has no file type of its own.
    pub fn label(self) -> &'static str {
        match self {
            Container::Best => "",
            Container::Mp4 => "MP4",
            Container::Mkv => "MKV",
            Container::Webm => "WebM",
        }
    }
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 4] = [AudioFormat::Best, AudioFormat::Mp3, AudioFormat::M4a, AudioFormat::Opus];

    /// The id saved nowhere but used in the editor's list; it is also the word yt-dlp's `--audio-format` takes.
    pub fn id(self) -> &'static str {
        match self {
            AudioFormat::Best => "best",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Opus => "opus",
        }
    }

    pub fn from_id(id: &str) -> Option<AudioFormat> {
        AudioFormat::ALL.into_iter().find(|f| f.id() == id)
    }

    /// The name shown in a list: a file type, the same in every language. Empty for `Best`, which has no file type of its own.
    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Best => "",
            AudioFormat::Mp3 => "MP3",
            AudioFormat::M4a => "M4A",
            AudioFormat::Opus => "Opus",
        }
    }
}

impl Preset {
    /// What the list shows beside the name: the container or the sound format (empty when the site decides it).
    pub fn badge(&self) -> &'static str {
        match &self.mode {
            Mode::Video { container, .. } => container.label(),
            Mode::Audio { format } => format.label(),
        }
    }
}

/// Make a saved list usable: only presets of its kind (`audio` or video), never empty, no two with the same id, no empty name.
pub fn repaired(list: Vec<Preset>, audio: bool) -> Vec<Preset> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Preset> = list
        .into_iter()
        .filter(|p| p.is_audio() == audio && !p.id.is_empty() && seen.insert(p.id.clone()))
        .map(|mut p| {
            if p.name.trim().is_empty() {
                p.name = p.id.clone();
            }
            p
        })
        .collect();
    if out.is_empty() {
        out = seeded(audio);
    }
    out
}

/// The list a fresh install holds: the video presets, or the audio ones.
pub fn seeded(audio: bool) -> Vec<Preset> {
    if audio {
        return vec![
            Preset::audio("audio-best", "Audio, best quality", AudioFormat::Best),
            Preset::audio("audio-mp3", "Audio MP3", AudioFormat::Mp3),
            Preset::audio("audio-m4a", "Audio M4A", AudioFormat::M4a),
            Preset::audio("audio-opus", "Audio Opus", AudioFormat::Opus),
        ];
    }
    vec![
        Preset::video("video-best", "Video, best quality", None, Container::Best),
        Preset::video("video-mp4-best", "Video, best quality (MP4)", None, Container::Mp4),
        Preset::video("video-mp4-1080", "Video 1080p (MP4)", Some(1080), Container::Mp4),
        Preset::video("video-mp4-720", "Video 720p (MP4)", Some(720), Container::Mp4),
        Preset::video("video-mkv-best", "Video, best quality (MKV)", None, Container::Mkv),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_seeded_list_has_unique_ids_and_only_its_kind() {
        for audio in [false, true] {
            let list = seeded(audio);
            let mut ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), list.len());
            assert!(list.iter().all(|p| p.is_audio() == audio));
        }
    }

    #[test]
    fn a_saved_list_is_never_empty_and_has_no_repeated_id() {
        assert_eq!(repaired(Vec::new(), false), seeded(false));
        assert_eq!(repaired(Vec::new(), true), seeded(true));
        let mut a = Preset::new("A", false);
        a.id = "same".to_owned();
        let mut b = Preset::new("B", false);
        b.id = "same".to_owned();
        let mut blank = Preset::new("  ", false);
        blank.id = "blank".to_owned();
        let sound = Preset::new("S", true);
        let list = repaired(vec![a, b, blank, sound], false);
        assert_eq!(list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["same", "blank"]);
        assert_eq!(list[1].name, "blank", "an empty name shows the id instead");
    }

    #[test]
    fn two_new_presets_never_share_an_id() {
        assert_ne!(Preset::new("x", false).id, Preset::new("x", true).id);
    }

    #[test]
    fn a_preset_survives_the_settings_file() {
        let p = Preset { id: "p".to_owned(), name: "n".to_owned(), mode: Mode::Video { max_height: Some(480), container: Container::Webm } };
        let back: Preset = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.badge(), "WebM");
        assert_eq!(back.args()[1], "res:480,vcodec:vp9,acodec:opus");
    }

    #[test]
    fn a_video_preset_limits_the_height_and_names_its_container() {
        let list: Vec<Preset> = seeded(false).into_iter().chain(seeded(true)).collect();
        let p = list.iter().find(|p| p.id == "video-mp4-720").unwrap();
        assert_eq!(p.args(), ["-S", "res:720,vcodec:h264,acodec:m4a", "--merge-output-format", "mp4"]);
        let best = list.iter().find(|p| p.id == "video-mp4-best").unwrap();
        assert_eq!(best.args()[1], "res,vcodec:h264,acodec:m4a", "no limit: the tallest one");
        let any = list.iter().find(|p| p.id == "video-best").unwrap();
        assert_eq!(any.args(), ["-S", "res"], "best: no codec asked for and no file type forced");
        assert_eq!(any.badge(), "");
        let capped = Preset { id: "c".to_owned(), name: "c".to_owned(), mode: Mode::Video { max_height: Some(1080), container: Container::Best } };
        assert_eq!(capped.args(), ["-S", "res:1080"]);
        let mkv = list.iter().find(|p| p.id == "video-mkv-best").unwrap();
        assert_eq!(mkv.args(), ["-S", "res", "--merge-output-format", "mkv"]);
    }

    #[test]
    fn an_audio_preset_extracts_the_sound_in_its_format() {
        let list: Vec<Preset> = seeded(false).into_iter().chain(seeded(true)).collect();
        let p = list.iter().find(|p| p.id == "audio-mp3").unwrap();
        let a = p.args();
        assert!(a.windows(2).any(|w| w == ["--audio-format", "mp3"]));
        assert!(a.contains(&"-x".to_owned()));
        let best = list.iter().find(|p| p.id == "audio-best").unwrap();
        assert!(best.args().windows(2).any(|w| w == ["--audio-format", "best"]), "best: the sound is not converted");
        assert_eq!(best.badge(), "");
    }
}
