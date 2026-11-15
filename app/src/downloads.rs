//! The window's one view: the links to fetch on the left, the run's choices in a column at the right, and Download.
//!
//! The same shape as the converters' batch view with the file list replaced by a list of links. Every row is ONE video: a link to a
//! playlist is opened into its videos before anything is fetched. The panel holds the choices of the whole run (preset, folder,
//! what to do when the file exists, how many at once); a row carries none of its own.
//!
//! ⚠ CLAUDE: NO HARDCODED UI STRINGS. Everything goes through the t() lookup with a literal key at the call site.

use crate::{
    catalog::UisFldLabelled,
    converters, i18n, links, manager,
    player::{Player, PlayerPane, Track},
    settings::{self, Settings},
    tools::{self, RUNNER, RunState, ToolPopup},
};
use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdFilm, LdFolder, LdFolderSearch, LdGauge, LdMusic, LdPause, LdPlay, LdRotateCw, LdVideo, LdVolume2, LdX},
};
use engine::runner::{Job, JobStatus, Overwrite, Tags};
use std::{collections::HashMap, path::PathBuf, time::Duration};

#[derive(Clone, PartialEq)]
enum RowState {
    /// yt-dlp is being asked what the link is.
    Looking(engine::probe::Progress),
    LookupFailed(String),
    Ready,
}

#[derive(Clone, PartialEq)]
struct Row {
    id: u64,
    url: String,
    title: String,
    /// Uploader and length, when the lookup gave them.
    sub: String,
    thumbnail: Option<String>,
    /// The source holds sound only: it is downloaded with an audio preset.
    audio: bool,
    /// The album and the track number, when the link was a set on a sound site.
    tags: Tags,
    /// The set (playlist) the link was listed in: a destination may put it in a sub-folder of its own.
    set: Option<engine::probe::SetInfo>,
    state: RowState,
    /// The engine's job, once Download has been pressed for this row.
    job: Option<u64>,
    /// The file a download of an earlier session made: the row comes back finished, with no job.
    done: Option<PathBuf>,
}

impl Row {
    /// The broom removes this row: it is finished AND it is in the list shown (the other list keeps its finished rows).
    fn clearable(&self, audio: bool, statuses: &HashMap<u64, JobStatus>) -> bool {
        self.audio == audio && matches!(self.status(statuses), Some(JobStatus::Done(_)))
    }

    /// The row's name as the list and the player show it: the track number first, when the file has one.
    fn name(&self) -> String {
        match self.tags.track {
            Some((n, of)) => format!("{n:0w$}  {}", self.title, w = of.to_string().len().max(2)),
            None => self.title.clone(),
        }
    }

    /// Where the row's download stands: its job's status, or "done" for a row that came back finished.
    fn status(&self, statuses: &HashMap<u64, JobStatus>) -> Option<JobStatus> {
        match self.job {
            Some(job) => statuses.get(&job).cloned(),
            None => self.done.clone().map(|file| JobStatus::Done(Some(file))),
        }
    }

    /// The row as it is kept for the next session. A download that is not finished is kept as the link alone.
    fn saved(&self, statuses: &HashMap<u64, JobStatus>) -> links::Saved {
        let looked = self.state == RowState::Ready;
        links::Saved {
            url: self.url.clone(),
            audio: self.audio,
            looked,
            title: if looked { self.title.clone() } else { String::new() },
            sub: self.sub.clone(),
            thumbnail: self.thumbnail.clone(),
            album: self.tags.album.clone(),
            track: self.tags.track,
            set: self.set.as_ref().map(|s| links::SavedSet { title: s.title.clone(), year: s.year.clone(), date: s.date.clone(), uploader: s.uploader.clone() }),
            done: match self.status(statuses) {
                Some(JobStatus::Done(file)) => file,
                _ => None,
            },
        }
    }

    /// A row of the last session whose lookup had answered. A finished one whose file is gone is a link to download again.
    fn restored(id: u64, s: links::Saved) -> Row {
        Row {
            id,
            url: s.url,
            title: s.title,
            sub: s.sub,
            thumbnail: s.thumbnail,
            audio: s.audio,
            tags: Tags { album: s.album, track: s.track },
            set: s.set.map(|s| engine::probe::SetInfo { title: s.title, year: s.year, date: s.date, uploader: s.uploader }),
            state: RowState::Ready,
            job: None,
            done: s.done.filter(|file| file.is_file()),
        }
    }
}

/// `3:07`, `1:02:09`.
fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// The address of every link in a pasted text: anything that starts with `http://` or `https://`, one per word.
fn links_in(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let word = word.trim_matches(|c: char| matches!(c, '<' | '>' | '"' | '\'' | ',' | ';'));
        if (word.starts_with("http://") || word.starts_with("https://")) && !found.iter().any(|f| f == word) {
            found.push(word.to_owned());
        }
    }
    found
}

fn percent_text(fraction: Option<f32>) -> String {
    fraction.map_or_else(|| "…".to_owned(), |f| format!("{}%", (f * 100.0).round() as u32))
}

/// A row can be handed to the engine: its link is understood, and it has no job running or done.
fn can_submit(row: &Row, statuses: &HashMap<u64, JobStatus>) -> bool {
    row.state == RowState::Ready
        && match row.status(statuses) {
            None => true,
            Some(JobStatus::Failed(_) | JobStatus::Cancelled) => true,
            Some(JobStatus::Queued | JobStatus::Running(_) | JobStatus::Done(_)) => false,
        }
}

#[component]
pub fn Downloads(settings: Signal<Settings>, run: RunState, lang: String, on_manage: EventHandler<String>, mut audio_list: Signal<bool>, add_clipboard: Signal<u32>, clear_request: Signal<u32>, mut can_clear: Signal<bool>) -> Element {
    let mut pick_nonce = use_signal(|| 0u32);
    // The downloads are frozen (Pause). Read from the runner when the view is made, so it survives a change of tab.
    let mut paused = use_signal(|| RUNNER.is_paused());
    // The lists of the last session come back: a link whose lookup had answered as its row, the others are looked up again below.
    let kept = use_hook(|| std::rc::Rc::new(links::load()));
    let mut rows = use_signal({
        let kept = kept.clone();
        move || kept.iter().filter(|s| s.looked).enumerate().map(|(n, s)| Row::restored(n as u64 + 1, s.clone())).collect::<Vec<Row>>()
    });
    let mut next_id = use_signal(|| rows.peek().len() as u64 + 1);
    let mut lookup_tick = use_signal(|| 0u32);
    let statuses = run.statuses;
    // One player per list: what plays under Video and what plays under Audio are kept apart, each with its file, its place and its picture.
    let video_player = Player::new();
    let audio_player = Player::new();
    // The converters a finished file can be opened in (the video one, the audio one), looked for once.
    let (video_converter, audio_converter) = use_hook(|| (converters::VIDEO.installed(), converters::AUDIO.installed()));
    // The button that opens a file in a converter carries that converter's own icon.
    let (video_icon, audio_icon) = use_hook(|| (converters::VIDEO.icon_src(), converters::AUDIO.icon_src()));

    // The picked traffic set: how many download at once, and how fast all of them may go together.
    use_effect(move || {
        let set = settings.read().traffic();
        RUNNER.set_parallel(set.parallel as usize);
        RUNNER.set_rate(set.limit_bytes());
    });
    // Off, no browser is ever touched; on, only a video that asks for a sign-in borrows the login of one the PC has.
    use_effect(move || RUNNER.set_login(settings.read().login.then(engine::browsers::installed)));
    // A cookies file the user picked is tried first, whether or not the browsers are on.
    use_effect(move || {
        let file = settings.read().cookies_file.trim().to_owned();
        RUNNER.set_cookies_file((!file.is_empty()).then(|| std::path::PathBuf::from(file)));
    });

    let t = |key: &str| i18n::t(&lang, key);

    // Look the links up: each becomes a row at once ("Analyzing"), then one row per video once yt-dlp has answered.
    let mut add_links = move |text: String, audio: bool| {
        for url in links_in(&text) {
            // A link already in THIS list is not added twice; the same link may be in both lists (its video and its sound).
            if rows.peek().iter().any(|r| r.url == url && r.audio == audio) {
                continue;
            }
            let id = *next_id.peek();
            next_id += 1;
            let progress = engine::probe::Progress::default();
            rows.write().push(Row { id, title: url.clone(), url: url.clone(), sub: String::new(), thumbnail: None, audio, tags: Tags::default(), set: None, state: RowState::Looking(progress.clone()), job: None, done: None });
            spawn(async move {
                // A lookup never runs while downloads do: asked by both at once, a site answers 403. Until it starts, the row reads "Queued".
                while statuses.peek().values().any(JobStatus::is_active) {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    if !rows.peek().iter().any(|r| r.id == id) {
                        return;
                    }
                }
                // yt-dlp may still be installing (a first launch): the lookup waits for it.
                let ytdlp = loop {
                    if let Some(p) = RUNNER.ytdlp() {
                        break p;
                    }
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    if !rows.peek().iter().any(|r| r.id == id) {
                        return;
                    }
                };
                let asked = url.clone();
                let lookup = tokio::task::spawn_blocking(move || RUNNER.lookup(&ytdlp, &asked, &progress));
                // The lookup is not a download, so nothing else repaints the window while it runs: the row's percentage needs a tick.
                while !lookup.is_finished() {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    lookup_tick += 1;
                }
                let answer = lookup.await.unwrap_or_else(|e| Err(e.to_string()));
                let Some(pos) = rows.peek().iter().position(|r| r.id == id) else { return };
                match answer {
                    Ok(items) => {
                        let fresh: Vec<Row> = items
                            .into_iter()
                            .filter(|i| !rows.peek().iter().any(|r| r.url == i.url && r.audio == audio && r.id != id))
                            .map(|i| {
                                let rid = *next_id.peek();
                                next_id += 1;
                                let sub = [i.uploader.clone(), i.duration.map(clock)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                                Row { id: rid, url: i.url, title: i.title, sub, thumbnail: i.thumbnail, audio, tags: Tags { album: i.album, track: i.track }, set: i.set, state: RowState::Ready, job: None, done: None }
                            })
                            .collect();
                        let mut w = rows.write();
                        w.remove(pos);
                        for (n, row) in fresh.into_iter().enumerate() {
                            w.insert(pos + n, row);
                        }
                    }
                    Err(reason) => {
                        rows.write()[pos].state = RowState::LookupFailed(reason);
                    }
                }
            });
        }
    };

    // A link the last session had not finished looking up is asked again, once, at the start.
    use_hook(move || {
        for s in kept.iter().filter(|s| !s.looked) {
            add_links(s.url.clone(), s.audio);
        }
    });
    // The lists are written each time they change (a row added, removed, looked up, downloaded): nothing waits for the app to close.
    use_effect(move || {
        let statuses = statuses.read();
        let saved: Vec<links::Saved> = rows.read().iter().map(|r| r.saved(&statuses)).collect();
        links::save(&saved);
    });

    // The + in the header asks for the links on the clipboard to be added (0 = not asked yet).
    use_effect(move || {
        if add_clipboard() == 0 {
            return;
        }
        let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
        add_links(text, *audio_list.peek());
    });

    let cfg = settings.read().clone();
    let video_id = cfg.preset_for(false).id;
    let audio_id = cfg.preset_for(true).id;
    // Each list (video, audio) has its own destination pick; the panel shows the one of the list shown.
    let on_audio = audio_list();
    let destination = cfg.destination(on_audio);
    let destination_id = destination.id.clone();
    let destinations = engine::destination::repaired(cfg.destinations.clone());
    let picked_once = cfg.one_off(on_audio).cloned();
    let replace = cfg.replace;
    let traffics = cfg.traffics.clone();
    let traffic_id = cfg.traffic().id;

    let _ = lookup_tick();
    let snapshot = statuses.read().clone();
    // The finished files of each list, in the order of the list: what its player steps through.
    let queue_of = |audio: bool| -> Vec<Track> {
        rows.read()
            .iter()
            .filter(|r| r.audio == audio)
            .filter_map(|r| match r.status(&snapshot) {
                Some(JobStatus::Done(Some(p))) => Some(Track { path: p, title: r.name(), video: !r.audio }),
                _ => None,
            })
            .collect()
    };
    let (video_queue, audio_queue) = (queue_of(false), queue_of(true));
    let busy = snapshot.values().any(JobStatus::is_active);
    // Whatever ends the downloads (the last one finishing) lets the runner forget the pause: follow it.
    let is_paused = paused() && busy;
    let waiting = rows.read().iter().filter(|r| r.audio == on_audio && can_submit(r, &snapshot)).count();
    let ready = (run.ready)();
    let shown_rows: Vec<Row> = rows.read().iter().filter(|r| r.audio == on_audio).cloned().collect();
    let have_rows = !shown_rows.is_empty();
    let done = snapshot.values().filter(|s| matches!(s, JobStatus::Done(_))).count();
    let failed = snapshot.values().filter(|s| matches!(s, JobStatus::Failed(_))).count();
    let any_over = rows.read().iter().any(|r| r.clearable(on_audio, &snapshot));

    let popup = tools::popup(&lang, &run);
    let mut dismissed = run.dismissed;

    let status_text = if !ready {
        t("msgMainToolsMissing")
    } else if done + failed > 0 && !busy {
        t("msgMainSummary").replace("{done}", &done.to_string()).replace("{failed}", &failed.to_string())
    } else {
        String::new()
    };

    // Hands rows to the engine: every row that can be (the Download button), or only the one asked for (its retry button).
    let mut submit = move |only: Option<u64>| {
        let cfg = settings.read().clone();
        // Download takes the list that is shown; the other one waits for its own turn.
        let list = *audio_list.peek();
        let destination = cfg.destination(list);
        let base = settings::folder_of(&destination);
        let snapshot = statuses.peek().clone();
        let mut w = rows.write();
        for row in w.iter_mut().filter(|r| r.audio == list && only.is_none_or(|id| r.id == id) && can_submit(r, &snapshot)) {
            if let Some(old) = row.job.take() {
                RUNNER.forget(old);
            }
            row.job = Some(RUNNER.submit(Job {
                url: row.url.clone(),
                preset: cfg.preset_for(row.audio),
                folder: destination.folder_for(&base, row.set.as_ref()),
                name: destination.template(row.tags.track),
                overwrite: if cfg.replace { Overwrite::Replace } else { Overwrite::Skip },
                tags: cfg.tag.then(|| row.tags.clone()),
                rate: None,
            }));
        }
    };
    let download = move |_| submit(None);

    let mut clear_done = move || {
        let snapshot = statuses.peek().clone();
        let shown = *audio_list.peek();
        rows.write().retain(|r| {
            let over = r.clearable(shown, &snapshot);
            if over {
                if let Some(j) = r.job {
                    RUNNER.forget(j);
                }
            }
            !over
        });
    };

    // The broom in the header asks for the finished rows to be removed; it is enabled while there are some.
    use_effect(move || {
        if clear_request() != 0 {
            clear_done();
        }
    });
    use_effect(use_reactive!(|any_over| can_clear.set(any_over)));

    let (s_video, s_video_tip) = (t("lblMainVideo"), t("tipMainPresetVideo"));
    let (s_audio, s_audio_tip) = (t("lblWorkAudio"), t("tipMainPresetAudio"));
    let s_edit_audio = t("tipMainEditAudio");
    let (s_folder, s_folder_tip, s_pick) = (t("lblMainFolder"), t("tipMainFolder"), t("lblMainDestPick"));
    let (s_edit_presets, s_edit_destinations) = (t("tipMainEditPresets"), t("tipMainEditDestinations"));
    let (s_over, s_over_tip) = (t("lblMainOverwrite"), t("tipMainOverwrite"));
    let (s_skip, s_replace) = (t("lblMainOverwriteSkip"), t("lblMainOverwriteReplace"));
    let s_show = t("tipMainShowFile");
    let s_open_video = t("tipMainOpenIn").replace("{app}", converters::VIDEO.name);
    let s_open_audio = t("tipMainOpenIn").replace("{app}", converters::AUDIO.name);
    let s_retry = t("tipMainRetry");
    let s_play = t("tipMainPlay");
    let (s_download, s_stop) = (t("tipMainDownload"), t("tipMainStop"));
    let (s_traffic, s_traffic_tip, s_edit_traffic) = (t("lblMainTraffic"), t("tipMainTraffic"), t("tipMainEditTraffic"));
    let (s_download_label, s_stop_label) = (t("btnMainDownload"), t("btnMainStop"));
    let (s_pause_label, s_resume_label) = (t("btnMainPause"), t("btnMainResume"));
    let (s_pause, s_resume) = (t("tipMainPauseAll"), t("tipMainResume"));
    let s_empty = t("msgMainListEmpty");
    let video_presets = engine::preset::repaired(cfg.presets.clone(), false);
    let audio_presets = engine::preset::repaired(cfg.audios.clone(), true);

    rsx! {
        div { class: "dl-body",
            div { class: "dl-main",
                if let Some(popup) = popup {
                    ToolPopup { lang: lang.clone(), popup, on_close: move |_| dismissed.set(true) }
                }
                // Both players stay mounted: the one of the other list waits out of sight, paused, and is there as it was when its list is back.
                if video_player.track.read().is_some() {
                    PlayerPane { player: video_player, queue: video_queue, lang: lang.clone(), away: on_audio }
                }
                if audio_player.track.read().is_some() {
                    PlayerPane { player: audio_player, queue: audio_queue, lang: lang.clone(), away: !on_audio }
                }
                div { class: "uis-lvw-scroll dl-list",
                    if !have_rows {
                        div { class: "dl-empty", "{s_empty}" }
                    }
                    for row in shown_rows {
                        {
                            let status = row.status(&snapshot);
                            let active = status.as_ref().is_some_and(JobStatus::is_active);
                            let (badge, detail, state, fraction) = match (&row.state, &status) {
                                (RowState::Looking(p), _) if !p.started() => (t("msgMainStatusQueued"), String::new(), "idle", None),
                                (RowState::Looking(p), _) => (
                                    p.fraction().map_or_else(|| t("msgMainStatusLooking"), |f| format!("{} {}", t("msgMainStatusLooking"), percent_text(Some(f)))),
                                    String::new(),
                                    "idle",
                                    None,
                                ),
                                (RowState::LookupFailed(reason), _) => (t("msgMainStatusLookupFailed"), reason.clone(), "err", None),
                                (RowState::Ready, None) => (String::new(), String::new(), "idle", None),
                                (RowState::Ready, Some(JobStatus::Queued)) => (t("msgMainStatusQueued"), String::new(), "idle", None),
                                (RowState::Ready, Some(JobStatus::Running(p))) if is_paused => (t("msgMainStatusPaused"), String::new(), "idle", p.fraction),
                                (RowState::Ready, Some(JobStatus::Running(p))) => (percent_text(p.fraction), String::new(), "run", p.fraction),
                                (RowState::Ready, Some(JobStatus::Done(path))) => {
                                    (t("msgMainStatusDone"), path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(), "ok", None)
                                }
                                (RowState::Ready, Some(JobStatus::Cancelled)) => (t("msgMainStatusCancelled"), String::new(), "idle", None),
                                (RowState::Ready, Some(JobStatus::Failed(reason))) => (t("msgMainStatusFailed"), reason.clone(), "err", None),
                            };
                            let id = row.id;
                            let job = row.job;
                            let retry_lookup = matches!(row.state, RowState::LookupFailed(_));
                            let retry_download = matches!(status, Some(JobStatus::Failed(_)));
                            let tip = if active { t("tipMainCancel") } else { t("tipMainRemove") };
                            let is_audio = row.audio;
                            let shown = match &status {
                                Some(JobStatus::Done(Some(p))) => Some(p.clone()),
                                _ => None,
                            };
                            let name = row.name();
                            // The row whose file is in the player (playing or paused) is the highlighted one.
                            let player = if is_audio { audio_player } else { video_player };
                            let row_class = if shown.as_deref().is_some_and(|f| player.is_open_on(f)) { "uis-lvw-item dl-row active" } else { "uis-lvw-item dl-row" };
                            let width = fraction.map_or(0, |f| (f * 100.0).round() as u32);
                            rsx! {
                                div { key: "{id}", class: row_class,
                                    div { class: "uis-lvw-item-icon dl-cover",
                                        if let Some(src) = &row.thumbnail {
                                            img { class: "dl-thumb", src: "{src}", draggable: false }
                                        } else if is_audio {
                                            Icon { width: 18, height: 18, icon: LdMusic }
                                        } else {
                                            Icon { width: 18, height: 18, icon: LdFilm }
                                        }
                                    }
                                    div { class: "uis-lvw-item-body",
                                        div { class: "uis-lvw-item-name", "{name}" }
                                        div { class: "uis-lvw-item-sub", "{row.sub}" }
                                        if fraction.is_some() {
                                            div { class: "dl-bar", div { class: "dl-bar-fill", style: "width: {width}%" } }
                                        }
                                        if state == "err" && !detail.is_empty() {
                                            div { class: "uis-lvw-item-sub dl-error", "{detail}" }
                                        } else if state == "ok" && !detail.is_empty() {
                                            div { class: "uis-lvw-item-sub", "{detail}" }
                                        }
                                    }
                                    if let Some(file) = shown.clone() {
                                        button {
                                            class: "uis-btn ico subtle",
                                            title: "{s_play}",
                                            "aria-label": "{s_play}",
                                            onclick: {
                                                let track = Track { path: file.clone(), title: name.clone(), video: !is_audio };
                                                move |_| player.play(track.clone())
                                            },
                                            if player.is_sounding(&file) {
                                                Icon { width: 14, height: 14, icon: LdPause }
                                            } else {
                                                Icon { width: 14, height: 14, icon: LdPlay }
                                            }
                                        }
                                        if let Some(program) = if is_audio { audio_converter.clone() } else { video_converter.clone() } {
                                            {
                                                let tip = if is_audio { s_open_audio.clone() } else { s_open_video.clone() };
                                                let file = file.clone();
                                                rsx! {
                                                    button {
                                                        class: "uis-btn ico subtle",
                                                        title: "{tip}",
                                                        "aria-label": "{tip}",
                                                        onclick: move |_| converters::open(&program, &file),
                                                        img { class: "dl-app-icon", src: if is_audio { audio_icon.clone() } else { video_icon.clone() }, draggable: false }
                                                    }
                                                }
                                            }
                                        }
                                        button {
                                            class: "uis-btn ico subtle",
                                            title: "{s_show}",
                                            "aria-label": "{s_show}",
                                            onclick: move |_| show_in_folder(&file),
                                            Icon { width: 14, height: 14, icon: LdFolderSearch }
                                        }
                                    } else if !badge.is_empty() {
                                        span { class: "uis-lvw-item-badge dl-state dl-state-{state}", "{badge}" }
                                    }
                                    if retry_lookup || retry_download {
                                        button {
                                            class: "uis-btn ico subtle",
                                            title: "{s_retry}",
                                            "aria-label": "{s_retry}",
                                            onclick: {
                                                let url = row.url.clone();
                                                move |_| {
                                                    if retry_lookup {
                                                        rows.write().retain(|r| r.id != id);
                                                        add_links(url.clone(), is_audio);
                                                    } else {
                                                        submit(Some(id));
                                                    }
                                                }
                                            },
                                            Icon { width: 14, height: 14, icon: LdRotateCw }
                                        }
                                    }
                                    button {
                                        class: "uis-btn ico subtle",
                                        title: "{tip}",
                                        "aria-label": "{tip}",
                                        onclick: move |_| {
                                            match job {
                                                Some(j) if active => RUNNER.cancel(j),
                                                other => {
                                                    if let Some(j) = other {
                                                        RUNNER.forget(j);
                                                    }
                                                    rows.write().retain(|r| r.id != id);
                                                }
                                            }
                                        },
                                        Icon { width: 14, height: 14, icon: LdX }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            aside { class: "uis-pnl dl-panel uis-cmp-panel uis-cmp-side",
                if !on_audio {
                UisFldLabelled { compact: true, label: s_video.clone(), title: s_video_tip.clone(),
                    select {
                        class: "uis-ddn-std",
                        title: "{s_video_tip}",
                        "aria-label": "{s_video_tip}",
                        value: "{video_id}",
                        onchange: move |e| settings.write().preset_id = e.value(),
                        for p in video_presets {
                            option { key: "{p.id}", value: "{p.id}", selected: p.id == video_id, "{p.name}" }
                        }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_edit_presets}",
                        "aria-label": "{s_edit_presets}",
                        onclick: move |_| on_manage.call(manager::PRESETS.to_owned()),
                        Icon { width: 14, height: 14, icon: LdVideo }
                    }
                }
                } else {
                UisFldLabelled { compact: true, label: s_audio.clone(), title: s_audio_tip.clone(),
                    select {
                        class: "uis-ddn-std",
                        title: "{s_audio_tip}",
                        "aria-label": "{s_audio_tip}",
                        value: "{audio_id}",
                        onchange: move |e| settings.write().audio_id = e.value(),
                        for p in audio_presets {
                            option { key: "{p.id}", value: "{p.id}", selected: p.id == audio_id, "{p.name}" }
                        }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_edit_audio}",
                        "aria-label": "{s_edit_audio}",
                        onclick: move |_| on_manage.call(manager::AUDIO.to_owned()),
                        Icon { width: 14, height: 14, icon: LdVolume2 }
                    }
                }
                }
                UisFldLabelled { compact: true, label: s_folder.clone(), title: s_folder_tip.clone(),
                    select {
                        class: "uis-ddn-std",
                        title: "{s_folder_tip}",
                        "aria-label": "{s_folder_tip}",
                        // A new key rebuilds the box after the folder picker closes: cancelled, it must not go on
                        // showing the pick entry as if it were the choice.
                        key: "{pick_nonce}",
                        value: "{destination_id}",
                        onchange: move |e| {
                            let chosen = e.value();
                            if chosen == engine::destination::PICK_ID {
                                // Owned by the app's window: opened from the drop-down's own pop-up, a dialog without an owner came up behind it.
                                let window = dioxus::desktop::window();
                                let start = settings::folder_of(&settings.peek().destination(on_audio));
                                spawn(async move {
                                    if let Some(dir) = rfd::AsyncFileDialog::new().set_parent(&window.window).set_directory(start).pick_folder().await {
                                        settings.write().pick_folder_once(on_audio, dir.path());
                                    }
                                    pick_nonce += 1;
                                });
                            } else {
                                settings.write().set_destination(on_audio, chosen);
                            }
                        },
                        for d in destinations {
                            option { key: "{d.id}", value: "{d.id}", selected: d.id == destination_id, "{d.name}" }
                        }
                        if let Some(one) = picked_once.as_ref() {
                            option { key: "{one.id}", value: "{one.id}", selected: one.id == destination_id, "{one.name}" }
                        }
                        option { value: "{engine::destination::PICK_ID}", "{s_pick}" }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_edit_destinations}",
                        "aria-label": "{s_edit_destinations}",
                        onclick: move |_| on_manage.call(manager::DESTINATIONS.to_owned()),
                        Icon { width: 14, height: 14, icon: LdFolder }
                    }
                }
                UisFldLabelled { compact: true, label: s_over.clone(), title: s_over_tip.clone(),
                    select {
                        class: "uis-ddn-std",
                        title: "{s_over_tip}",
                        "aria-label": "{s_over_tip}",
                        value: if replace { "replace" } else { "skip" },
                        onchange: move |e| settings.write().replace = e.value() == "replace",
                        option { value: "skip", selected: !replace, "{s_skip}" }
                        option { value: "replace", selected: replace, "{s_replace}" }
                    }
                }
                UisFldLabelled { compact: true, label: s_traffic.clone(), title: s_traffic_tip.clone(),
                    select {
                        class: "uis-ddn-std",
                        title: "{s_traffic_tip}",
                        "aria-label": "{s_traffic_tip}",
                        value: "{traffic_id}",
                        onchange: move |e| settings.write().traffic_id = e.value(),
                        for tr in traffics {
                            option { key: "{tr.id}", value: "{tr.id}", selected: tr.id == traffic_id, "{tr.name}" }
                        }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_edit_traffic}",
                        "aria-label": "{s_edit_traffic}",
                        onclick: move |_| on_manage.call(manager::TRAFFIC.to_owned()),
                        Icon { width: 14, height: 14, icon: LdGauge }
                    }
                }
                div { class: "dl-panel-run uis-cmp-run",
                    // Download; while the downloads run, Pause (Resume when paused) beside Stop.
                    if busy {
                        if is_paused {
                            button {
                                class: "uis-btn",
                                title: "{s_resume}",
                                onclick: move |_| {
                                    RUNNER.resume();
                                    paused.set(false);
                                },
                                "{s_resume_label}"
                            }
                        } else {
                            button {
                                class: "uis-btn",
                                title: "{s_pause}",
                                onclick: move |_| {
                                    RUNNER.pause();
                                    paused.set(RUNNER.is_paused());
                                },
                                "{s_pause_label}"
                            }
                        }
                        button {
                            class: "uis-btn primary",
                            title: "{s_stop}",
                            onclick: move |_| {
                                RUNNER.cancel_all();
                                paused.set(false);
                            },
                            "{s_stop_label}"
                        }
                    } else {
                        button {
                            class: "uis-btn primary",
                            title: "{s_download}",
                            disabled: !ready || waiting == 0,
                            onclick: download,
                            "{s_download_label}"
                        }
                    }
                }
            }
        }
        div { class: "uis-barh-footer",
            span { class: "dl-status", "{status_text}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_text_gives_its_links_once_each() {
        let text = "see https://a.test/1 and <https://b.test/2>,\nhttps://a.test/1\nnot a link http:/c.test ftp://d.test";
        assert_eq!(links_in(text), ["https://a.test/1", "https://b.test/2"]);
        assert!(links_in("").is_empty());
    }

    #[test]
    fn a_length_reads_as_a_clock() {
        assert_eq!(clock(19), "0:19");
        assert_eq!(clock(187), "3:07");
        assert_eq!(clock(3729), "1:02:09");
    }

    #[test]
    fn only_a_row_that_is_understood_and_not_running_or_done_can_be_submitted() {
        let row = |state, job| Row { id: 1, url: "u".into(), title: "t".into(), sub: String::new(), thumbnail: None, audio: false, tags: Tags::default(), set: None, state, job, done: None };
        let mut st = std::collections::HashMap::new();
        assert!(can_submit(&row(RowState::Ready, None), &st));
        // A row that came back finished from an earlier session is done, with no job: it is not downloaded again,
        // and it is saved with its file.
        let kept = Row { done: Some(PathBuf::from("kept.mp3")), ..row(RowState::Ready, None) };
        assert!(!can_submit(&kept, &st));
        assert_eq!(kept.saved(&st).done, Some(PathBuf::from("kept.mp3")));
        assert!(kept.saved(&st).looked);
        // A link still being looked up is saved as the link alone, to be asked again.
        let asking = row(RowState::Looking(Default::default()), None).saved(&st);
        assert!(!asking.looked && asking.title.is_empty());
        assert!(!can_submit(&row(RowState::Looking(Default::default()), None), &st));
        assert!(!can_submit(&row(RowState::LookupFailed("x".into()), None), &st));
        st.insert(7, JobStatus::Failed("boom".into()));
        assert!(can_submit(&row(RowState::Ready, Some(7)), &st), "a failed one can be tried again");
        st.insert(7, JobStatus::Done(None));
        assert!(!can_submit(&row(RowState::Ready, Some(7)), &st));
        st.insert(7, JobStatus::Queued);
        assert!(!can_submit(&row(RowState::Ready, Some(7)), &st));
    }

    #[test]
    fn the_broom_clears_only_the_finished_rows_of_the_list_shown() {
        let row = |audio, job| Row { id: 1, url: "u".into(), title: "t".into(), sub: String::new(), thumbnail: None, audio, tags: Tags::default(), set: None, state: RowState::Ready, job, done: None };
        let mut st = std::collections::HashMap::new();
        st.insert(1, JobStatus::Done(None));
        st.insert(2, JobStatus::Queued);
        let (video, sound) = (row(false, Some(1)), row(true, Some(1)));
        assert!(video.clearable(false, &st) && !video.clearable(true, &st), "a finished video is cleared from the Video list only");
        assert!(sound.clearable(true, &st) && !sound.clearable(false, &st), "a finished sound is cleared from the Audio list only");
        assert!(!row(false, Some(2)).clearable(false, &st), "a row that is not finished stays");
        let kept = Row { done: Some(PathBuf::from("kept.mp3")), ..row(true, None) };
        assert!(kept.clearable(true, &st), "a row that came back finished is cleared too");
    }
}

/// Open the file manager with the finished file selected (its folder where the platform cannot select).
fn show_in_folder(file: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let arg = format!("/select,\"{}\"", file.display());
        if std::process::Command::new("explorer").raw_arg(arg).spawn().is_ok() {
            return;
        }
    }
    if let Some(dir) = file.parent() {
        let _ = open::that(dir);
    }
}
