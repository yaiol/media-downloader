//! The player of the finished files: a bar at the foot of the window, as in the music tools (play, stop, previous, next, the title, a
//! position slider with the time, the volume). A row's play icon opens it on that file; when a file ends the next finished one follows.

use crate::{i18n, settings};
use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdPause, LdPlay, LdSkipBack, LdSkipForward, LdSquare, LdVolume1, LdVolume2, LdVolumeX, LdX},
};
use engine::player::Sound;
use std::{path::PathBuf, time::Duration};

/// A finished file the player can play.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
}

/// What the rows and the bar share: the file on the bar (none = no bar), whether it is meant to be sounding, and where it is in seconds.
#[derive(Clone, Copy, PartialEq)]
pub struct Player {
    pub track: Signal<Option<Track>>,
    pub playing: Signal<bool>,
    pub position: Signal<f64>,
}

impl Player {
    pub fn new() -> Self {
        Self { track: use_signal(|| None), playing: use_signal(|| false), position: use_signal(|| 0.0) }
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

    /// Is this file the one sounding now?
    pub fn is_sounding(&self, path: &std::path::Path) -> bool {
        *self.playing.read() && self.track.read().as_ref().is_some_and(|t| t.path == path)
    }
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// The file after `current` in `queue`, and the one before it.
fn neighbours(queue: &[Track], current: Option<&Track>) -> (Option<Track>, Option<Track>) {
    let Some(i) = current.and_then(|c| queue.iter().position(|q| q.path == c.path)) else { return (None, None) };
    (i.checked_sub(1).and_then(|p| queue.get(p).cloned()), queue.get(i + 1).cloned())
}

#[component]
pub fn PlayerBar(player: Player, queue: ReadSignal<Vec<Track>>, lang: String) -> Element {
    let t = |key: &str| i18n::t(&lang, key);
    let Player { mut track, mut playing, mut position } = player;
    let mut sound = use_signal(|| None::<Sound>);
    let mut length = use_signal(|| 0.0_f64);
    let mut volume = use_signal(|| 1.0_f32);
    let mut scrub = use_signal(|| None::<f64>);
    // Bumped by a seek, so the sound starts again from the new place.
    let mut seeked = use_signal(|| 0u32);
    let mut failure = use_signal(String::new);

    // The file changed: its length.
    use_effect(move || {
        let t = track();
        length.set(t.and_then(|t| engine::player::duration(&t.path)).unwrap_or(0.0));
    });

    // What is meant to sound against what does: start it from where it is, or stop it where it got to.
    let lang_fx = lang.clone();
    use_effect(move || {
        let (t, want, _) = (track(), playing(), seeked());
        sound.write().take();
        let Some(t) = t.filter(|_| want) else { return };
        let ffmpeg = engine::ffmpeg::locate(settings::ffmpeg_dir().as_deref());
        let started = ffmpeg.ok_or_else(|| i18n::t(&lang_fx, "msgStgFfmpegNone")).and_then(|ff| Sound::start(&ff, &t.path, *position.peek(), *volume.peek()));
        match started {
            Ok(s) => {
                failure.set(String::new());
                sound.set(Some(s));
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
            let state = sound.peek().as_ref().map(|s| (s.position(), s.finished()));
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
    let (t_play, t_pause, t_stop, t_prev, t_next, t_vol, t_close) =
        (t("tipMainPlay"), t("tipMainPause"), t("tipMainStop"), t("tipMainPrevious"), t("tipMainNext"), t("tipMainVolume"), t("tipMainPlayerClose"));
    let title = track.read().as_ref().map(|t| t.title.clone()).unwrap_or_default();
    let shown = scrub().unwrap_or_else(|| position());
    let len = length();
    let vol = volume();
    let sounding = playing();
    let error = failure();
    let t_toggle = if sounding { t_pause } else { t_play };
    let (prev_go, next_go) = (prev.clone(), next.clone());

    rsx! {
        button {
            class: "uis-btn ico small subtle",
            title: "{t_prev}",
            "aria-label": "{t_prev}",
            disabled: prev.is_none(),
            onclick: move |_| {
                if let Some(p) = prev_go.clone() {
                    player.go_to(p);
                }
            },
            Icon { width: 14, height: 14, icon: LdSkipBack }
        }
        button {
            class: "uis-btn ico primary",
            title: "{t_toggle}",
            "aria-label": "{t_toggle}",
            onclick: move |_| {
                if let Some(s) = sound.peek().as_ref() {
                    position.set(s.position());
                }
                let now = *playing.peek();
                playing.set(!now);
            },
            if sounding {
                Icon { width: 14, height: 14, icon: LdPause }
            } else {
                Icon { width: 14, height: 14, icon: LdPlay }
            }
        }
        button {
            class: "uis-btn ico small subtle",
            title: "{t_stop}",
            "aria-label": "{t_stop}",
            onclick: move |_| {
                position.set(0.0);
                playing.set(false);
            },
            Icon { width: 14, height: 14, icon: LdSquare }
        }
        button {
            class: "uis-btn ico small subtle",
            title: "{t_next}",
            "aria-label": "{t_next}",
            disabled: next.is_none(),
            onclick: move |_| {
                if let Some(n) = next_go.clone() {
                    player.go_to(n);
                }
            },
            Icon { width: 14, height: 14, icon: LdSkipForward }
        }
        span { class: "dl-player-title", title: "{title}", "{title}" }
        if !error.is_empty() {
            span { class: "dl-status dl-error", "{error}" }
        }
        input {
            class: "dl-seek",
            r#type: "range",
            min: "0",
            max: "{len.max(1.0)}",
            step: "0.1",
            value: "{shown}",
            disabled: len <= 0.0,
            oninput: move |e| {
                if let Ok(v) = e.value().parse::<f64>() {
                    scrub.set(Some(v));
                }
            },
            onchange: move |e| {
                if let Ok(v) = e.value().parse::<f64>() {
                    position.set(v);
                    seeked += 1;
                }
                scrub.set(None);
            },
        }
        span { class: "uis-hnt dl-player-time", "{clock(shown)} / {clock(len)}" }
        div { class: "uis-barh-sep" }
        if vol <= 0.0 {
            Icon { width: 14, height: 14, icon: LdVolumeX }
        } else if vol < 0.5 {
            Icon { width: 14, height: 14, icon: LdVolume1 }
        } else {
            Icon { width: 14, height: 14, icon: LdVolume2 }
        }
        input {
            class: "dl-vol",
            r#type: "range",
            min: "0",
            max: "1",
            step: "0.02",
            value: "{vol}",
            title: "{t_vol}",
            "aria-label": "{t_vol}",
            oninput: move |e| {
                if let Ok(v) = e.value().parse::<f32>() {
                    volume.set(v);
                    if let Some(s) = sound.peek().as_ref() {
                        s.set_volume(v);
                    }
                }
            },
        }
        button {
            class: "uis-btn ico small subtle",
            title: "{t_close}",
            "aria-label": "{t_close}",
            onclick: move |_| {
                playing.set(false);
                track.set(None);
            },
            Icon { width: 14, height: 14, icon: LdX }
        }
    }
}
