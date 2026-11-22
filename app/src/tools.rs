//! The two programs the app runs - yt-dlp, which downloads, and ffmpeg, which yt-dlp merges the picture and the sound with - and
//! the one downloader that drives them.
//!
//! Both are looked up in the folder every yaiol app shares (never on PATH) and again after an install. Nothing installed is a real
//! state the window reports instead of failing later at Download. Whatever starts by itself (the installs at launch, the updates,
//! the follow-up of a run) lives in `RunState::follow`, never in a view, or it runs again every time the view is mounted.
//!
//! ⚠ NO HARDCODED UI STRINGS. Everything goes through the t() lookup with a literal key at the call site.

use crate::{i18n, settings};
use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdCheck, LdDownload, LdRefreshCw},
};
use engine::{
    deno,
    ffmpeg_install::{self, Download, DownloadState},
    runner::{JobStatus, Runner, Tools},
    ytdlp::{self, Install},
};
use std::{collections::HashMap, sync::{Arc, LazyLock}, time::Duration};

/// yt-dlp installs itself and updates itself; the engine's runner asks `YTDLP` for a newer one when a download fails.
pub static YTDLP: LazyLock<Install> = LazyLock::new(Install::new);
/// Which yt-dlp is the latest; asked at launch, and again from Settings only when that gave no answer.
pub static YTDLP_LATEST: LazyLock<ytdlp::Latest> = LazyLock::new(ytdlp::Latest::new);
/// The ffmpeg install, the same code the converters run.
pub static FFMPEG: LazyLock<Download> = LazyLock::new(Download::new);
/// Which ffmpeg yaiol says is the compatible one.
pub static FFMPEG_LATEST: LazyLock<ffmpeg_install::Latest> = LazyLock::new(ffmpeg_install::Latest::new);

/// Deno, the script runtime yt-dlp reads YouTube with: installed and kept current the same way as yt-dlp.
pub static DENO: LazyLock<Install> = LazyLock::new(deno::installer);
/// Which Deno is the latest.
pub static DENO_LATEST: LazyLock<ytdlp::Latest> = LazyLock::new(deno::latest);

pub static RUNNER: LazyLock<Runner> = LazyLock::new(|| {
    let runner = Runner::new(Arc::new(|| {
        // A download failed: look for a newer yt-dlp (the window shows "Updating yt-dlp" while this runs).
        let dir = settings::ytdlp_dir().ok_or_else(|| "no folder for yt-dlp".to_owned())?;
        YTDLP.update(&dir)
    }));
    refresh_tools(&runner);
    runner
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Ytdlp,
    Ffmpeg,
    Deno,
}

/// Look for the programs again; true when yt-dlp and ffmpeg were found (Deno is only for YouTube: the rest works without it).
pub fn refresh_tools(runner: &Runner) -> bool {
    let ytdlp = ytdlp::locate(settings::ytdlp_dir().as_deref());
    let ffmpeg_dir = settings::ffmpeg_dir();
    let ffmpeg = engine::ffmpeg::locate(ffmpeg_dir.as_deref());
    engine::log::set_context("yt-dlp", &ytdlp.as_ref().map_or_else(|| "not installed".to_owned(), |p| p.display().to_string()));
    engine::log::set_context("ffmpeg", &ffmpeg.as_ref().map_or_else(|| "not installed".to_owned(), |p| p.display().to_string()));
    let deno = deno::locate(settings::deno_dir().as_deref());
    engine::log::set_context("deno", &deno.as_ref().map_or_else(|| "not installed".to_owned(), |p| p.display().to_string()));
    let tools = ytdlp.zip(ffmpeg).map(|(ytdlp, _)| Tools { ytdlp, ffmpeg_dir, deno });
    let ready = tools.is_some();
    runner.set_tools(tools);
    ready
}

pub fn ytdlp_installed() -> Option<String> {
    settings::ytdlp_dir().and_then(|dir| ytdlp::installed_version(&dir))
}

pub fn ffmpeg_installed() -> Option<String> {
    settings::ffmpeg_dir().and_then(|dir| ffmpeg_install::installed_version(&dir))
}

pub fn deno_installed() -> Option<String> {
    settings::deno_dir().and_then(|dir| deno::installed_version(&dir))
}

fn ffmpeg_is_current(manifest: &ffmpeg_install::Manifest) -> bool {
    settings::ffmpeg_dir().is_some_and(|dir| ffmpeg_install::is_current(&dir, manifest))
}

pub fn install(tool: Tool) {
    match tool {
        Tool::Ytdlp => {
            if let Some(dir) = settings::ytdlp_dir() {
                YTDLP.start(dir);
            }
        }
        Tool::Ffmpeg => {
            if let Some(dir) = settings::ffmpeg_dir() {
                FFMPEG.start(dir);
            }
        }
        Tool::Deno => {
            if let Some(dir) = settings::deno_dir() {
                DENO.start(dir);
            }
        }
    }
}

/// What the app knows about the runs and about its programs, whichever part of the window is on screen. ONE copy, made in the app.
#[derive(Clone, Copy, PartialEq)]
pub struct RunState {
    pub statuses: Signal<HashMap<u64, JobStatus>>,
    pub ytdlp: Signal<DownloadState>,
    pub ffmpeg: Signal<DownloadState>,
    pub deno: Signal<DownloadState>,
    pub ready: Signal<bool>,
    /// The popup the user closed (a failure): it stays closed until an install starts again.
    pub dismissed: Signal<bool>,
}

impl RunState {
    /// A hook: call it once, from a component that outlives everything it feeds.
    pub fn new() -> RunState {
        let statuses = use_signal(HashMap::<u64, JobStatus>::new);
        let ytdlp = use_signal(|| DownloadState::Idle);
        let ffmpeg = use_signal(|| DownloadState::Idle);
        let deno = use_signal(|| DownloadState::Idle);
        let ready = use_signal(|| RUNNER.has_tools());
        let dismissed = use_signal(|| false);
        let state = RunState { statuses, ytdlp, ffmpeg, deno, ready, dismissed };
        use_hook(|| {
            // Which versions are the latest. Asked on every machine: the app cannot keep its programs right without the
            // answer, so this is not the optional request the dev-machine switch exists to silence.
            YTDLP_LATEST.check();
            FFMPEG_LATEST.check();
            DENO_LATEST.check();
            // Nothing installed: install it now, there is nothing for the user to decide.
            if ytdlp_installed().is_none() {
                install(Tool::Ytdlp);
            }
            if ffmpeg_installed().is_none() {
                install(Tool::Ffmpeg);
            }
            if deno_installed().is_none() {
                install(Tool::Deno);
            }
        });
        use_future(move || state.follow());
        state
    }

    /// The window polls the runner and the installs; the engine never calls back into it. Only a CHANGED snapshot is written, so
    /// an idle queue does not re-render the list five times a second.
    async fn follow(self) {
        let RunState { mut statuses, mut ytdlp, mut ffmpeg, mut deno, mut ready, mut dismissed } = self;
        // A newer approved version is installed by itself, once per launch (the popup says "Updating").
        let (mut ytdlp_update_tried, mut ffmpeg_update_tried, mut deno_update_tried) = (false, false, false);
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let snap: HashMap<u64, JobStatus> = RUNNER.snapshot().into_iter().collect();
            if *statuses.peek() != snap {
                statuses.set(snap);
            }
            let (y, f, d) = (YTDLP.snapshot(), FFMPEG.snapshot(), DENO.snapshot());
            if (y.is_active() || f.is_active() || d.is_active()) && *dismissed.peek() {
                dismissed.set(false);
            }
            let mut changed = false;
            if *ytdlp.peek() != y {
                changed |= matches!(y, DownloadState::Done(_));
                ytdlp.set(y);
            }
            if *ffmpeg.peek() != f {
                changed |= matches!(f, DownloadState::Done(_));
                ffmpeg.set(f);
            }
            if *deno.peek() != d {
                changed |= matches!(d, DownloadState::Done(_));
                deno.set(d);
            }
            if changed {
                ready.set(refresh_tools(&RUNNER));
            }
            if let ytdlp::LatestState::Known(tag) = YTDLP_LATEST.snapshot() {
                if !ytdlp_update_tried && !YTDLP.snapshot().is_active() && ytdlp_installed().is_some_and(|have| have != tag) {
                    ytdlp_update_tried = true;
                    install(Tool::Ytdlp);
                }
            }
            if let ytdlp::LatestState::Known(tag) = DENO_LATEST.snapshot() {
                if !deno_update_tried && !DENO.snapshot().is_active() && deno_installed().is_some_and(|have| have != tag) {
                    deno_update_tried = true;
                    install(Tool::Deno);
                }
            }
            if let ffmpeg_install::LatestState::Known(m) = FFMPEG_LATEST.snapshot() {
                if !ffmpeg_update_tried && !FFMPEG.snapshot().is_active() && ffmpeg_installed().is_some() && !ffmpeg_is_current(&m) {
                    ffmpeg_update_tried = true;
                    install(Tool::Ffmpeg);
                }
            }
        }
    }
}

/// What the popup says; each carries its text, and whether the program is already in place (an update, not a first install).
#[derive(Clone, PartialEq)]
pub enum Popup {
    /// An install is running: no button, it closes by itself when it ends.
    Progress { tool: Tool, text: String, update: bool },
    /// An install failed: try again, or close.
    Failed { tool: Tool, text: String, update: bool },
}

/// The popup to show now, if any: the install that is running, else the one that failed and was not closed.
pub fn popup(lang: &str, run: &RunState) -> Option<Popup> {
    for (tool, state, have) in [
        (Tool::Ytdlp, (run.ytdlp)(), ytdlp_installed().is_some()),
        (Tool::Ffmpeg, (run.ffmpeg)(), ffmpeg_installed().is_some()),
        (Tool::Deno, (run.deno)(), deno_installed().is_some()),
    ] {
        if state.is_active() {
            if let Some(text) = note(lang, tool, &state) {
                return Some(Popup::Progress { tool, text, update: have });
            }
        }
    }
    if *run.dismissed.peek() {
        return None;
    }
    for (tool, state) in [(Tool::Ytdlp, (run.ytdlp)()), (Tool::Ffmpeg, (run.ffmpeg)()), (Tool::Deno, (run.deno)())] {
        if let DownloadState::Failed(_) = state {
            if let Some(text) = note(lang, tool, &state) {
                let have = match tool {
                    Tool::Ytdlp => ytdlp_installed().is_some(),
                    Tool::Ffmpeg => ffmpeg_installed().is_some(),
                    Tool::Deno => deno_installed().is_some(),
                };
                return Some(Popup::Failed { tool, text, update: have });
            }
        }
    }
    None
}

/// What the window says about an install that is running or has failed; `None` when nothing is happening.
pub fn note(lang: &str, tool: Tool, state: &DownloadState) -> Option<String> {
    let t = |key: &str| i18n::t(lang, key);
    let mb = |n: u64| (n / (1024 * 1024)).to_string();
    match (tool, state) {
        (Tool::Ffmpeg, DownloadState::Downloading { done, total }) => {
            Some(t("msgMainDownloading").replace("{done}", &mb(*done)).replace("{total}", &total.map_or_else(|| "?".to_owned(), mb)))
        }
        (Tool::Ytdlp, DownloadState::Downloading { done, total }) => {
            Some(t("msgMainYtdlpDownloading").replace("{done}", &mb(*done)).replace("{total}", &total.map_or_else(|| "?".to_owned(), mb)))
        }
        (Tool::Deno, DownloadState::Downloading { done, total }) => {
            Some(t("msgMainDenoDownloading").replace("{done}", &mb(*done)).replace("{total}", &total.map_or_else(|| "?".to_owned(), mb)))
        }
        (Tool::Deno, DownloadState::Verifying | DownloadState::Unpacking) => Some(t("msgMainDenoFinishing")),
        (Tool::Deno, DownloadState::Failed(reason)) => Some(t("msgMainDenoFailed").replace("{reason}", reason)),
        (Tool::Ffmpeg, DownloadState::Verifying | DownloadState::Unpacking) => Some(t("msgMainDownloadFinishing")),
        (Tool::Ytdlp, DownloadState::Verifying | DownloadState::Unpacking) => Some(t("msgMainYtdlpFinishing")),
        (Tool::Ffmpeg, DownloadState::Failed(reason)) => Some(t("msgMainDownloadFailed").replace("{reason}", reason)),
        (Tool::Ytdlp, DownloadState::Failed(reason)) => Some(t("msgMainYtdlpFailed").replace("{reason}", reason)),
        (_, DownloadState::Idle | DownloadState::Done(_)) => None,
    }
}

#[component]
pub fn ToolPopup(lang: String, popup: Popup, on_close: EventHandler<()>) -> Element {
    let t = |key: &str| i18n::t(&lang, key);
    let (retry, close) = (t("btnMainFfmpegRetry"), t("btnMainFfmpegClose"));
    let (tool, update, text, action) = match &popup {
        Popup::Progress { tool, text, update } => (*tool, *update, text.clone(), None),
        Popup::Failed { tool, text, update } => (*tool, *update, text.clone(), Some(retry)),
    };
    let title = match (tool, update) {
        (Tool::Ytdlp, false) => t("ttlMainYtdlpInstalling"),
        (Tool::Ytdlp, true) => t("ttlMainYtdlpUpdating"),
        (Tool::Ffmpeg, false) => t("ttlMainFfmpegInstalling"),
        (Tool::Ffmpeg, true) => t("ttlMainFfmpegUpdating"),
        (Tool::Deno, false) => t("ttlMainDenoInstalling"),
        (Tool::Deno, true) => t("ttlMainDenoUpdating"),
    };
    rsx! {
        div { class: "uis-dl-backdrop",
            div { class: "uis-dlp",
                Icon { icon: LdDownload, class: "uis-dlp-icon" }
                div { class: "uis-dlp-title", "{title}" }
                div { class: "uis-dlp-sub", "{text}" }
                if let Some(action) = action {
                    div { class: "uis-dlp-foot",
                        button { class: "uis-btn", onclick: move |_| install(tool), "{action}" }
                        button { class: "uis-btn", onclick: move |_| on_close.call(()), "{close}" }
                    }
                }
            }
        }
    }
}

/// Settings > General: which version of each program is installed, and the one button that brings it up to date.
#[component]
pub fn ToolsSetting(lang: String) -> Element {
    let mut ytdlp_have = use_signal(ytdlp_installed);
    let mut ffmpeg_have = use_signal(ffmpeg_installed);
    let mut y_latest = use_signal(|| YTDLP_LATEST.snapshot());
    let mut f_latest = use_signal(|| FFMPEG_LATEST.snapshot());
    let mut y_dl = use_signal(|| YTDLP.snapshot());
    let mut f_dl = use_signal(|| FFMPEG.snapshot());
    let mut deno_have = use_signal(deno_installed);
    let mut d_latest = use_signal(|| DENO_LATEST.snapshot());
    let mut d_dl = use_signal(|| DENO.snapshot());
    // The answer comes from the check at launch; it is asked again only when that one gave none (offline then).
    use_hook(|| {
        if !matches!(YTDLP_LATEST.snapshot(), ytdlp::LatestState::Known(_)) {
            YTDLP_LATEST.check();
        }
        if !matches!(FFMPEG_LATEST.snapshot(), ffmpeg_install::LatestState::Known(_)) {
            FFMPEG_LATEST.check();
        }
        if !matches!(DENO_LATEST.snapshot(), ytdlp::LatestState::Known(_)) {
            DENO_LATEST.check();
        }
    });
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let (yl, fl) = (YTDLP_LATEST.snapshot(), FFMPEG_LATEST.snapshot());
            if *y_latest.peek() != yl {
                y_latest.set(yl);
            }
            if *f_latest.peek() != fl {
                f_latest.set(fl);
            }
            let dl = DENO_LATEST.snapshot();
            if *d_latest.peek() != dl {
                d_latest.set(dl);
            }
            let dd = DENO.snapshot();
            if *d_dl.peek() != dd {
                if matches!(dd, DownloadState::Done(_)) {
                    deno_have.set(deno_installed());
                }
                d_dl.set(dd);
            }
            let (yd, fd) = (YTDLP.snapshot(), FFMPEG.snapshot());
            if *y_dl.peek() != yd {
                if matches!(yd, DownloadState::Done(_)) {
                    ytdlp_have.set(ytdlp_installed());
                }
                y_dl.set(yd);
            }
            if *f_dl.peek() != fd {
                if matches!(fd, DownloadState::Done(_)) {
                    ffmpeg_have.set(ffmpeg_installed());
                }
                f_dl.set(fd);
            }
        }
    });

    let t = |key: &str| i18n::t(&lang, key);

    // yt-dlp
    let y_have = ytdlp_have();
    let y_latest = y_latest();
    let y_up_to_date = matches!(&y_latest, ytdlp::LatestState::Known(tag) if y_have.as_deref() == Some(tag.as_str()));
    let y_latest_line = match &y_latest {
        ytdlp::LatestState::Unknown | ytdlp::LatestState::Checking => t("msgStgYtdlpChecking"),
        ytdlp::LatestState::Failed(_) => t("msgStgYtdlpCheckFailed"),
        ytdlp::LatestState::Known(tag) => t("msgStgYtdlpLatest").replace("{version}", tag),
    };
    let y_explain = match &y_latest {
        ytdlp::LatestState::Known(tag) => Some(t("msgStgYtdlpExplain").replace("{version}", tag)),
        _ => None,
    };
    let y_line = match &y_have {
        Some(v) => t("msgStgYtdlpInstalled").replace("{version}", v),
        None => t("msgStgYtdlpNone"),
    };
    let y_offer = matches!(y_latest, ytdlp::LatestState::Known(_)) && !y_up_to_date;
    let y_busy = y_dl().is_active();
    let y_button = if y_have.is_some() { t("btnStgToolUpdate") } else { t("btnStgToolDownload") };
    let y_note = note(&lang, Tool::Ytdlp, &y_dl());

    // ffmpeg
    let f_have = ffmpeg_have();
    let f_latest = f_latest();
    let f_up_to_date = matches!(&f_latest, ffmpeg_install::LatestState::Known(m) if f_have.is_some() && ffmpeg_is_current(m));
    let f_latest_line = match &f_latest {
        ffmpeg_install::LatestState::Unknown | ffmpeg_install::LatestState::Checking => t("msgStgFfmpegChecking"),
        ffmpeg_install::LatestState::Failed(_) => t("msgStgFfmpegCheckFailed"),
        ffmpeg_install::LatestState::Known(m) => t("msgStgFfmpegLatest").replace("{version}", &m.version),
    };
    let f_explain = match &f_latest {
        ffmpeg_install::LatestState::Known(m) => m.build().map(|b| {
            t("msgStgFfmpegExplain")
                .replace("{version}", &m.version)
                .replace("{size}", &(b.size / (1024 * 1024)).to_string())
                .replace("{source}", &b.source)
        }),
        _ => None,
    };
    let f_line = match &f_have {
        Some(v) => t("msgStgFfmpegInstalled").replace("{version}", v),
        None => t("msgStgFfmpegNone"),
    };
    let f_offer = matches!(f_latest, ffmpeg_install::LatestState::Known(_)) && !f_up_to_date;
    let f_busy = f_dl().is_active();
    let f_button = if f_have.is_some() { t("btnMainUpdateFfmpeg") } else { t("btnMainDownloadFfmpeg") };
    let f_note = note(&lang, Tool::Ffmpeg, &f_dl());

    // Deno
    let d_have = deno_have();
    let d_latest = d_latest();
    let d_up_to_date = matches!(&d_latest, ytdlp::LatestState::Known(tag) if d_have.as_deref() == Some(tag.as_str()));
    let d_latest_line = match &d_latest {
        ytdlp::LatestState::Unknown | ytdlp::LatestState::Checking => t("msgStgDenoChecking"),
        ytdlp::LatestState::Failed(_) => t("msgStgDenoCheckFailed"),
        ytdlp::LatestState::Known(tag) => t("msgStgDenoLatest").replace("{version}", tag),
    };
    let d_explain = match &d_latest {
        ytdlp::LatestState::Known(tag) => Some(t("msgStgDenoExplain").replace("{version}", tag)),
        _ => None,
    };
    let d_line = match &d_have {
        Some(v) => t("msgStgDenoInstalled").replace("{version}", v),
        None => t("msgStgDenoNone"),
    };
    let d_offer = matches!(d_latest, ytdlp::LatestState::Known(_)) && !d_up_to_date;
    let d_busy = d_dl().is_active();
    let d_button = if d_have.is_some() { t("btnStgToolUpdate") } else { t("btnStgToolDownload") };
    let d_note = note(&lang, Tool::Deno, &d_dl());
    let tools_label = t("lblStgTools");

    rsx! {
        div { class: "uis-fld-labelled divider",
            label { class: "uis-fld-labelled-label", "{tools_label}" }
            span {
                "{y_line}"
                if y_up_to_date {
                    " "
                    Icon { width: 14, height: 14, icon: LdCheck }
                }
            }
            if !y_up_to_date {
                span { "{y_latest_line}" }
            }
            if y_offer {
                div {
                    button { class: "uis-btn", disabled: y_busy, onclick: move |_| install(Tool::Ytdlp),
                        if y_have.is_some() {
                            Icon { width: 14, height: 14, icon: LdRefreshCw }
                        } else {
                            Icon { width: 14, height: 14, icon: LdDownload }
                        }
                        " {y_button}"
                    }
                }
                if let Some(text) = y_explain {
                    span { "{text}" }
                }
            }
            if let Some(text) = y_note {
                span { "{text}" }
            }
            span {
                "{f_line}"
                if f_up_to_date {
                    " "
                    Icon { width: 14, height: 14, icon: LdCheck }
                }
            }
            if !f_up_to_date {
                span { "{f_latest_line}" }
            }
            if f_offer {
                div {
                    button { class: "uis-btn", disabled: f_busy, onclick: move |_| install(Tool::Ffmpeg),
                        if f_have.is_some() {
                            Icon { width: 14, height: 14, icon: LdRefreshCw }
                        } else {
                            Icon { width: 14, height: 14, icon: LdDownload }
                        }
                        " {f_button}"
                    }
                }
                if let Some(text) = f_explain {
                    span { "{text}" }
                }
            }
            if let Some(text) = f_note {
                span { "{text}" }
            }
            span {
                "{d_line}"
                if d_up_to_date {
                    " "
                    Icon { width: 14, height: 14, icon: LdCheck }
                }
            }
            if !d_up_to_date {
                span { "{d_latest_line}" }
            }
            if d_offer {
                div {
                    button { class: "uis-btn", disabled: d_busy, onclick: move |_| install(Tool::Deno),
                        if d_have.is_some() {
                            Icon { width: 14, height: 14, icon: LdRefreshCw }
                        } else {
                            Icon { width: 14, height: 14, icon: LdDownload }
                        }
                        " {d_button}"
                    }
                }
                if let Some(text) = d_explain {
                    span { "{text}" }
                }
            }
            if let Some(text) = d_note {
                span { "{text}" }
            }
        }
    }
}
