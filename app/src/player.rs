//! The player of the finished files: a pane above the list, laid out as the preview of the converters is (the title, the picture of a
//! video, the times, the position, then one bar of buttons: previous, play, stop, next, the volume, close). A row's play icon opens it
//! on that file; when a file ends the next finished one follows. The pane starts the picture of a video with its sound.

use crate::{
    catalog::{UisPlayer, UisPlayerLabels},
    i18n, settings,
};
use dioxus::prelude::*;
use engine::{
    picture::{self, Channel, Stream},
    player::Sound,
};
use std::{path::PathBuf, rc::Rc, time::Duration};

/// A finished file the player can play.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    /// The file has a picture: the player shows it while it plays.
    pub video: bool,
}

/// What the rows and the bar share: the file on the bar (none = no bar), whether it is meant to be sounding, and where it is in seconds.
#[derive(Clone, Copy, PartialEq)]
pub struct Player {
    pub track: Signal<Option<Track>>,
    pub playing: Signal<bool>,
    pub position: Signal<f64>,
    /// Where the pictures of a playing video go, and the address the view's image reads them from.
    pub channel: Signal<Rc<Channel>>,
}

impl Player {
    pub fn new() -> Self {
        Self { track: use_signal(|| None), playing: use_signal(|| false), position: use_signal(|| 0.0), channel: use_signal(|| Rc::new(Channel::new())) }
    }

    /// The play icon of a row: the file on the bar is paused or resumed, any other file takes the bar from its start.
    pub fn play(mut self, track: Track) {
        if self.track.peek().as_ref().is_some_and(|t| t.path == track.path) {
            let now = *self.playing.peek();
            self.playing.set(!now);
        } else {
            self.go_to(track);
        }
    }

    /// Put `track` on the bar and start it from its beginning.
    fn go_to(mut self, track: Track) {
        self.position.set(0.0);
        self.track.set(Some(track));
        self.playing.set(true);
    }

    /// The address of the picture of the video on the player; None when it holds no video (or the picture server could not start).
    fn picture(&self) -> Option<String> {
        self.track.read().as_ref().filter(|t| t.video)?;
        self.channel.read().url.clone().ok()
    }

    /// Is this file the one in the player, sounding or paused?
    pub fn is_open_on(&self, path: &std::path::Path) -> bool {
        self.track.read().as_ref().is_some_and(|t| t.path == path)
    }

    /// Is this file the one sounding now?
    pub fn is_sounding(&self, path: &std::path::Path) -> bool {
        *self.playing.read() && self.track.read().as_ref().is_some_and(|t| t.path == path)
    }
}

/// The file after `current` in `queue`, and the one before it.
fn neighbours(queue: &[Track], current: Option<&Track>) -> (Option<Track>, Option<Track>) {
    let Some(i) = current.and_then(|c| queue.iter().position(|q| q.path == c.path)) else { return (None, None) };
    (i.checked_sub(1).and_then(|p| queue.get(p).cloned()), queue.get(i + 1).cloned())
}

/// `away`: the list shown is not the one the file belongs to. The pane is then not drawn and the file is paused, and it comes back
/// where it was when its own list is shown again.
#[component]
pub fn PlayerPane(player: Player, queue: ReadSignal<Vec<Track>>, lang: String, away: bool) -> Element {
    let t = |key: &str| i18n::t(&lang, key);
    let Player { mut track, mut playing, mut position, channel } = player;
    let mut sound = use_signal(|| None::<Sound>);
    let mut stream = use_signal(|| None::<Stream>);
    let mut length = use_signal(|| 0.0_f64);
    let mut volume = use_signal(|| 1.0_f32);
    // Bumped by a seek, so the sound starts again from the new place.
    let mut seeked = use_signal(|| 0u32);
    let mut failure = use_signal(String::new);

    // The other list is shown: the file waits, paused, until its own list is back.
    use_effect(use_reactive!(|away| {
        if away && *playing.peek() {
            playing.set(false);
        }
    }));

    // The file changed: its length.
    use_effect(move || {
        let t = track();
        // The tag reader knows the sound files and MP4; ffmpeg is asked for the rest (MKV, WebM).
        let long = t.and_then(|t| {
            engine::player::duration(&t.path).or_else(|| engine::ffmpeg::locate(settings::ffmpeg_dir().as_deref()).and_then(|ff| picture::duration(&ff, &t.path)))
        });
        length.set(long.unwrap_or(0.0));
    });

    // What is meant to sound against what does: start it from where it is, or stop it where it got to.
    let lang_fx = lang.clone();
    use_effect(move || {
        let (t, want, _) = (track(), playing(), seeked());
        sound.write().take();
        stream.write().take();
        let Some(t) = t else { return };
        let ffmpeg = engine::ffmpeg::locate(settings::ffmpeg_dir().as_deref());
        let at = *position.peek();
        if !want {
            // Paused or stopped: a video shows the picture of the moment it is at.
            if let (true, Some(ff)) = (t.video, ffmpeg) {
                let hub = channel.peek().hub.clone();
                std::thread::spawn(move || {
                    if let Err(e) = picture::still(&ff, &t.path, at, &hub) {
                        engine::log::error("player picture", &format!("{e}
input: {}", t.path.display()));
                    }
                });
            }
            return;
        }
        let Some(ff) = ffmpeg else {
            failure.set(i18n::t(&lang_fx, "msgStgFfmpegNone"));
            playing.set(false);
            return;
        };
        // The picture first, then the sound: both read the file from the same moment. A video with no sound plays its picture alone.
        let pictures = if t.video { Stream::start(&ff, &t.path, at, channel.peek().hub.clone()).ok() } else { None };
        match Sound::start(&ff, &t.path, at, *volume.peek()) {
            Ok(s) => {
                failure.set(String::new());
                sound.set(Some(s));
                stream.set(pictures);
            }
            Err(_) if pictures.is_some() => {
                failure.set(String::new());
                stream.set(pictures);
            }
            Err(e) => {
                engine::log::error("player", &e);
                failure.set(e);
                playing.set(false);
            }
        }
    });

    // Follow the playing: the position, and the end of the file (which hands the bar to the next one).
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            // A video is over when its picture is; its sound may be shorter, or missing.
            let pictures = stream.peek().as_ref().map(Stream::finished);
            let state = match (sound.peek().as_ref().map(|s| (s.position(), s.finished())), pictures) {
                (Some((p, ended)), Some(over)) => Some((p, over && ended)),
                (None, Some(over)) => Some((*position.peek(), over)),
                (heard, None) => heard,
            };
            match state {
                Some((p, false)) => position.set(p),
                Some((_, true)) => {
                    let (_, next) = neighbours(&queue.peek(), track.peek().as_ref());
                    match next {
                        Some(n) => player.go_to(n),
                        None => {
                            position.set(0.0);
                            playing.set(false);
                        }
                    }
                }
                None => {}
            }
        }
    });

    let (prev, next) = neighbours(&queue.read(), track.read().as_ref());
    let labels = UisPlayerLabels {
        play: t("tipMainPlay"),
        pause: t("tipMainPause"),
        stop: t("tipMainStop"),
        previous: t("tipMainPrevious"),
        next: t("tipMainNext"),
        volume: t("tipMainVolume"),
        close: t("tipMainPlayerClose"),
    };
    let title = track.read().as_ref().map(|t| t.title.clone()).unwrap_or_default();
    let (has_prev, has_next) = (prev.is_some(), next.is_some());
    let picture = player.picture();
    let box_class = match (away, picture.is_some()) {
        (true, _) => "dl-player dl-player-away",
        (false, true) => "dl-player dl-player-video",
        (false, false) => "dl-player",
    };

    // The box is the app's (how tall, its padding and border); what is in it is the catalog's player.
    rsx! {
        div { class: box_class,
            UisPlayer {
                title,
                error: failure(),
                picture,
                position: position(),
                length: length(),
                volume: volume(),
                playing: playing(),
                has_prev,
                has_next,
                labels,
                on_seek: move |v: f64| {
                    position.set(v);
                    seeked += 1;
                },
                on_toggle: move |_| {
                    if let Some(s) = sound.peek().as_ref() {
                        position.set(s.position());
                    }
                    let now = *playing.peek();
                    playing.set(!now);
                },
                on_stop: move |_| {
                    position.set(0.0);
                    playing.set(false);
                },
                on_prev: move |_| {
                    if let Some(p) = prev.clone() {
                        player.go_to(p);
                    }
                },
                on_next: move |_| {
                    if let Some(n) = next.clone() {
                        player.go_to(n);
                    }
                },
                on_volume: move |v: f32| {
                    volume.set(v);
                    if let Some(s) = sound.peek().as_ref() {
                        s.set_volume(v);
                    }
                },
                on_close: move |_| {
                    playing.set(false);
                    track.set(None);
                },
            }
        }
    }
}
