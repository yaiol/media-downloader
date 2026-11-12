//! The downloads: a queue of jobs, a few running at once, each one a yt-dlp process whose progress is read from its output.
//!
//! The window polls `snapshot`; the engine never calls back.
//!
//! **A failed download asks for a newer yt-dlp** (PLAN-media-downloader.md, decision 5): when a site changes, yt-dlp breaks until it
//! is updated, so a failure is the moment to look. The first failure of a batch runs the `updater` once; if it installed something
//! the failed job (and every job that failed with the old one) is tried once more. A job is retried at most once, and the
//! updater runs at most once per batch, so a link that is truly broken cannot loop and a queue of twenty asks GitHub once.
//!
//! **A video that wants a signed-in visitor is retried with a sign-in** (an age-restricted or members-only video): when a failure says
//! so, the same link is tried again with the cookies file the user picked (`set_cookies_file`), then with `--cookies-from-browser <browser>`
//! for each browser the PC has in turn, the one that worked last first. Only that failure triggers it, and the browsers are off unless the
//! window turned them on (`set_login`), so no browser is touched otherwise.

use crate::{
    browsers::{needs_login, Signin},
    preset::Preset,
    probe::Item,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsString,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overwrite {
    /// A file that is already there is left alone (and the job counts as done).
    Skip,
    Replace,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub url: String,
    pub preset: Preset,
    pub folder: PathBuf,
    /// The yt-dlp output template of the file's name inside the folder (`destination::template`).
    pub name: String,
    pub overwrite: Overwrite,
    /// Tag the file (an audio preset only): `None` = leave the tags alone.
    pub tags: Option<Tags>,
}

/// What a tagged file is given beyond what yt-dlp itself writes (title, artist, genre, date, cover): the album and the track's place in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub album: Option<String>,
    /// The track's number and how many there are.
    pub track: Option<(u32, u32)>,
}

/// A literal as yt-dlp's `--parse-metadata` reads its source: `\` and `:` are escaped, `%` is doubled.
fn parse_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace(':', "\\:").replace('%', "%%")
}

impl Tags {
    /// The yt-dlp options that tag an audio file. yt-dlp writes the tags itself (`--embed-metadata`), with ONE change: the page address is
    /// copied into the `purl` tag and then emptied, so it is not written over the file's own `comment` (a SoundCloud file made with Suno
    /// carries its id there). A tag whose source is empty is not written, and the file keeps its own. It happens at the `post_process`
    /// stage, just before the tags are written, so nothing else yt-dlp does is affected. The comment is settled afterwards by
    /// `aiid::finish`, which reads what the file kept. An opus has no comment field: ffmpeg stores the comment as `DESCRIPTION`, the same
    /// field the site's description would be written to, so for an opus the description is emptied too (it would erase the comment).
    /// `opus` is also true when the format is the site's own (`AudioFormat::Best`), because that file may be an opus.
    pub fn args(&self, opus: bool) -> Vec<String> {
        let mut a: Vec<String> = ["--embed-metadata", "--embed-thumbnail"].iter().map(|s| (*s).to_owned()).collect();
        let mut rules = vec!["post_process:%(webpage_url)s:%(meta_purl)s", "post_process::(?P<webpage_url>)"];
        if opus {
            rules.push("post_process::(?P<description>)");
        }
        for rule in rules {
            a.push("--parse-metadata".to_owned());
            a.push(rule.to_owned());
        }
        if let Some(album) = self.album.as_deref().filter(|s| !s.trim().is_empty()) {
            a.push("--parse-metadata".to_owned());
            a.push(format!("{}:%(album)s", parse_literal(album)));
        }
        if let Some((n, of)) = self.track {
            a.push("--parse-metadata".to_owned());
            a.push(format!("{n}/{of}:%(meta_track)s"));
        }
        a
    }
}

/// The programs a job needs: yt-dlp, the folder of the ffmpeg it merges the picture and the sound with, and the Deno it reads YouTube with.
#[derive(Clone, Debug)]
pub struct Tools {
    pub ytdlp: PathBuf,
    pub ffmpeg_dir: Option<PathBuf>,
    /// The Deno the app installed: yt-dlp runs YouTube's script with it. Without it a YouTube video may not download.
    pub deno: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Progress {
    /// 0..=1 of the file being fetched now (a video and its sound are two files, so it starts again once).
    pub fraction: Option<f32>,
    /// Bytes a second.
    pub speed: Option<u64>,
    /// Seconds left.
    pub eta: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Queued,
    Running(Progress),
    /// The file that was written, when yt-dlp said where.
    Done(Option<PathBuf>),
    Failed(String),
    Cancelled,
}

impl JobStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, JobStatus::Queued | JobStatus::Running(_))
    }
}

/// Look for a newer yt-dlp and install it: `Ok(true)` when one was installed, `Ok(false)` when the latest was already there.
pub type Updater = Arc<dyn Fn() -> Result<bool, String> + Send + Sync>;

#[derive(Default)]
struct Shared {
    next_id: u64,
    jobs: HashMap<u64, Job>,
    status: HashMap<u64, JobStatus>,
    order: Vec<u64>,
    queue: VecDeque<u64>,
    workers: usize,
    parallel: usize,
    tools: Option<Tools>,
    pids: HashMap<u64, u32>,
    cancelled: HashSet<u64>,
}

#[derive(Default)]
struct Gate {
    /// The updater has run in this batch.
    tried: bool,
    /// How many times it installed something; a job compares it with the number it started under.
    epoch: u64,
}

/// The browsers a sign-in may be borrowed from, and the one that worked last.
#[derive(Default)]
struct Login {
    /// `None` = off: no browser is ever touched.
    browsers: Option<Vec<String>>,
    worked: Option<String>,
    /// A cookies file the user picked; tried before any browser, whether or not the browsers are on.
    file: Option<PathBuf>,
}

#[derive(Clone)]
pub struct Runner {
    shared: Arc<Mutex<Shared>>,
    gate: Arc<Mutex<Gate>>,
    login: Arc<Mutex<Login>>,
    updater: Updater,
}

enum Failure {
    Cancelled,
    Error(String),
}

/// How many lookups are asking a site right now. A download does not start while there is one: a site asked by both at once answers 403.
static LOOKING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A lookup is under way for as long as this lives.
struct Looking;

impl Looking {
    fn begin() -> Looking {
        LOOKING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Looking
    }
}

impl Drop for Looking {
    fn drop(&mut self) {
        LOOKING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Runner {
    pub fn new(updater: Updater) -> Runner {
        Runner { shared: Arc::new(Mutex::new(Shared { parallel: 2, ..Shared::default() })), gate: Arc::new(Mutex::new(Gate::default())), login: Arc::new(Mutex::new(Login::default())), updater }
    }

    /// The programs, once they are installed (and again after an update); `None` while they are not.
    pub fn set_tools(&self, tools: Option<Tools>) {
        lock(&self.shared).tools = tools;
    }

    pub fn has_tools(&self) -> bool {
        lock(&self.shared).tools.is_some()
    }

    /// The yt-dlp program, once it is installed.
    pub fn ytdlp(&self) -> Option<PathBuf> {
        lock(&self.shared).tools.as_ref().map(|t| t.ytdlp.clone())
    }

    /// Let a download that needs a signed-in visitor borrow the login of one of these browsers (yt-dlp's names, in the order to try);
    /// `None` turns it off.
    pub fn set_login(&self, browsers: Option<Vec<String>>) {
        lock(&self.login).browsers = browsers;
    }

    /// The cookies file the user picked, if any (`None` or a file that is gone = none). Tried before the browsers.
    pub fn set_cookies_file(&self, file: Option<PathBuf>) {
        lock(&self.login).file = file;
    }

    /// The sign-ins to try: the cookies file first, then the browsers, the one that worked last first. Empty when there is nothing to try.
    fn login_order(&self) -> Vec<Signin> {
        let l = lock(&self.login);
        let mut order: Vec<Signin> = Vec::new();
        if let Some(file) = l.file.as_ref().filter(|f| f.is_file()) {
            order.push(Signin::File(file.clone()));
        }
        if let Some(list) = &l.browsers {
            let first: Vec<&String> = l.worked.iter().filter(|w| list.contains(w)).collect();
            let rest: Vec<&String> = list.iter().filter(|b| !first.contains(b)).collect();
            order.extend(first.into_iter().chain(rest).map(|b| Signin::Browser(b.clone())));
        }
        order
    }

    fn login_worked(&self, signin: &Signin) {
        if let Signin::Browser(b) = signin {
            lock(&self.login).worked = Some(b.clone());
        }
    }

    /// Look a link up (see `probe::probe`); when the site wants a sign-in, again with each sign-in in turn. One link at a time: several pasted
    /// at once wait for their turn, because a site that is asked for every set together answers 403.
    pub fn lookup(&self, ytdlp: &std::path::Path, url: &str, progress: &crate::probe::Progress) -> Result<Vec<Item>, String> {
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let _looking = Looking::begin();
        let deno = lock(&self.shared).tools.as_ref().and_then(|t| t.deno.clone());
        progress.start();
        let first = crate::probe::probe(ytdlp, deno.as_deref(), url, None, Some(progress));
        let Err(reason) = &first else { return first };
        if !needs_login(reason) {
            return first;
        }
        for signin in self.login_order() {
            if let Ok(items) = crate::probe::probe(ytdlp, deno.as_deref(), url, Some(&signin), Some(progress)) {
                self.login_worked(&signin);
                return Ok(items);
            }
        }
        first
    }

    /// How many download at once.
    pub fn set_parallel(&self, n: usize) {
        lock(&self.shared).parallel = n.max(1);
    }

    /// Queue a job and start working if a place is free. Returns the job's number.
    pub fn submit(&self, job: Job) -> u64 {
        let start_worker = {
            let mut s = lock(&self.shared);
            s.next_id += 1;
            let id = s.next_id;
            s.jobs.insert(id, job);
            s.status.insert(id, JobStatus::Queued);
            s.order.push(id);
            s.queue.push_back(id);
            let start = s.workers < s.parallel;
            if start {
                s.workers += 1;
            }
            (id, start)
        };
        if start_worker.1 {
            let this = self.clone();
            std::thread::spawn(move || this.work());
        }
        start_worker.0
    }

    pub fn snapshot(&self) -> Vec<(u64, JobStatus)> {
        let s = lock(&self.shared);
        s.order.iter().filter_map(|id| s.status.get(id).map(|st| (*id, st.clone()))).collect()
    }

    pub fn status(&self, id: u64) -> Option<JobStatus> {
        lock(&self.shared).status.get(&id).cloned()
    }

    /// Stop one job: a queued one never starts, a running one is killed.
    pub fn cancel(&self, id: u64) {
        let pid = {
            let mut s = lock(&self.shared);
            if !s.status.get(&id).is_some_and(JobStatus::is_active) {
                return;
            }
            s.cancelled.insert(id);
            s.queue.retain(|q| *q != id);
            if s.pids.contains_key(&id) {
                s.pids.get(&id).copied()
            } else {
                s.status.insert(id, JobStatus::Cancelled);
                None
            }
        };
        if let Some(pid) = pid {
            kill_tree(pid);
        }
    }

    pub fn cancel_all(&self) {
        let ids: Vec<u64> = lock(&self.shared).status.iter().filter(|(_, st)| st.is_active()).map(|(id, _)| *id).collect();
        for id in ids {
            self.cancel(id);
        }
    }

    /// Forget a job that is over (a row was removed from the list). A job still running is left alone.
    pub fn forget(&self, id: u64) {
        let mut s = lock(&self.shared);
        if s.status.get(&id).is_some_and(JobStatus::is_active) {
            return;
        }
        s.status.remove(&id);
        s.jobs.remove(&id);
        s.cancelled.remove(&id);
        s.order.retain(|o| *o != id);
    }

    fn work(&self) {
        loop {
            // A job waits (it stays "Queued") for the lookups under way to end.
            while LOOKING.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            let next = {
                let mut s = lock(&self.shared);
                match s.queue.pop_front() {
                    Some(id) => {
                        s.status.insert(id, JobStatus::Running(Progress::default()));
                        Some(id)
                    }
                    None => {
                        s.workers -= 1;
                        None
                    }
                }
            };
            let Some(id) = next else {
                // The batch is over when nothing is queued and nobody works: the next failure may ask for an update again.
                let over = {
                    let s = lock(&self.shared);
                    s.workers == 0 && s.queue.is_empty()
                };
                if over {
                    lock(&self.gate).tried = false;
                }
                return;
            };
            self.execute(id);
        }
    }

    fn execute(&self, id: u64) {
        let (job, tools) = {
            let s = lock(&self.shared);
            (s.jobs.get(&id).cloned(), s.tools.clone())
        };
        let (Some(job), Some(tools)) = (job, tools) else {
            self.finish(id, Err(Failure::Error("yt-dlp is not installed".to_owned())));
            return;
        };
        // Taking the gate also waits for an update that another job has under way, so this one starts with the new yt-dlp.
        let epoch0 = lock(&self.gate).epoch;
        let mut result = self.attempt(id, &job, &tools, None);
        match &result {
            Err(Failure::Error(reason)) if needs_login(reason) => {
                // A newer yt-dlp does not sign anyone in: borrow a browser's login instead.
                if let Some(again) = self.retry_with_login(id, &job, &tools) {
                    result = again;
                }
            }
            Err(Failure::Error(_)) if self.update_helps(epoch0) => {
                // The update may have replaced the program; the same tools struct names the same path.
                result = self.attempt(id, &job, &tools, None);
            }
            _ => {}
        }
        self.finish(id, result);
    }

    /// A job failed because the site wants a signed-in visitor: try each browser's login in turn. `None` when it is off or none worked,
    /// so the caller keeps the real error.
    fn retry_with_login(&self, id: u64, job: &Job, tools: &Tools) -> Option<Result<Option<PathBuf>, Failure>> {
        for signin in self.login_order() {
            match self.attempt(id, job, tools, Some(&signin)) {
                Ok(path) => {
                    self.login_worked(&signin);
                    return Some(Ok(path));
                }
                Err(Failure::Cancelled) => return Some(Err(Failure::Cancelled)),
                Err(Failure::Error(_)) => {}
            }
        }
        None
    }

    /// A job failed under the yt-dlp of epoch `epoch0`: ask for a newer one if this batch has not yet, and say whether one arrived.
    fn update_helps(&self, epoch0: u64) -> bool {
        let mut g = lock(&self.gate);
        if !g.tried {
            g.tried = true;
            match (self.updater)() {
                Ok(true) => g.epoch += 1,
                Ok(false) => {}
                Err(e) => crate::log::error("yt-dlp update after a failure", &e),
            }
        }
        g.epoch > epoch0
    }

    fn finish(&self, id: u64, result: Result<Option<PathBuf>, Failure>) {
        let mut s = lock(&self.shared);
        s.pids.remove(&id);
        let cancelled = s.cancelled.remove(&id);
        let status = match result {
            _ if cancelled => JobStatus::Cancelled,
            Ok(path) => JobStatus::Done(path),
            Err(Failure::Cancelled) => JobStatus::Cancelled,
            Err(Failure::Error(e)) => JobStatus::Failed(e),
        };
        s.status.insert(id, status);
    }

    fn attempt(&self, id: u64, job: &Job, tools: &Tools, signin: Option<&Signin>) -> Result<Option<PathBuf>, Failure> {
        if lock(&self.shared).cancelled.contains(&id) {
            return Err(Failure::Cancelled);
        }
        let args = build_args(job, tools, signin);
        let mut child = crate::ffmpeg::command(&tools.ytdlp)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Failure::Error(format!("yt-dlp could not start: {e}")))?;
        {
            let mut s = lock(&self.shared);
            s.pids.insert(id, child.id());
            // Cancelled between the check and the start: the process is already ours to stop.
            if s.cancelled.contains(&id) {
                drop(s);
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Cancelled);
            }
        }
        // What yt-dlp says about its own failures goes to its error output; read on the side so a full pipe never stops it.
        let stderr = child.stderr.take().map(|e| {
            std::thread::spawn(move || {
                let mut lines = Vec::new();
                for line in lossy_lines(e) {
                    lines.push(line);
                    if lines.len() > 60 {
                        lines.remove(0);
                    }
                }
                lines
            })
        });
        let mut path = None;
        if let Some(out) = child.stdout.take() {
            for line in lossy_lines(out) {
                if let Some(p) = parse_progress(&line) {
                    let mut s = lock(&self.shared);
                    if s.status.get(&id).is_some_and(JobStatus::is_active) {
                        s.status.insert(id, JobStatus::Running(p));
                    }
                } else if let Some(file) = line.strip_prefix("FILE:") {
                    path = Some(PathBuf::from(file.trim()));
                }
            }
        }
        let status = child.wait().map_err(|e| Failure::Error(e.to_string()))?;
        let said = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
        if lock(&self.shared).cancelled.contains(&id) {
            return Err(Failure::Cancelled);
        }
        if status.success() {
            // The comment was left alone by yt-dlp: a Suno id in it goes to YAIOL_AIID, any other comment becomes the page address.
            // A failure here never fails the download: the file is there, as yt-dlp left it, and the log says what happened.
            if let (Some(_), true, Some(file)) = (&job.tags, job.preset.is_audio(), &path) {
                if let Err(e) = crate::aiid::finish(file, &job.url) {
                    crate::log::error("tags", &format!("{}: {e}", file.display()));
                }
            }
            return Ok(path);
        }
        let reason = said.iter().rev().find(|l| l.contains("ERROR")).or(said.last()).map(|l| l.trim().trim_start_matches("ERROR:").trim().to_owned());
        let reason = reason.filter(|r| !r.is_empty()).unwrap_or_else(|| format!("yt-dlp stopped ({status})"));
        crate::log::error("download", &format!("{}\n{}\n{}", crate::log::command_line(&tools.ytdlp, &args), said.join("\n"), status));
        Err(Failure::Error(reason))
    }
}

/// Stop a process and what it started (yt-dlp starts ffmpeg to merge).
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let _ = crate::ffmpeg::command(std::path::Path::new("taskkill")).args(["/PID", &pid.to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    #[cfg(not(windows))]
    {
        // The children first, then the process itself.
        let _ = std::process::Command::new("pkill").args(["-TERM", "-P", &pid.to_string()]).status();
        let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
}

/// Every argument of one yt-dlp run. The program's own config files are ignored, so a machine's setup cannot change what a job does.
pub fn build_args(job: &Job, tools: &Tools, signin: Option<&Signin>) -> Vec<OsString> {
    // The cache is yt-dlp's own folder beside it: the default one (the user's home) may be unwritable.
    let mut a: Vec<OsString> = vec![OsString::from("--ignore-config"), OsString::from("--cache-dir"), crate::ytdlp::cache_dir(&tools.ytdlp).into_os_string()];
    let mut push = |s: &str| a.push(OsString::from(s));
    // A playlist was already opened into its videos by the probe: a link here is one video.
    push("--no-playlist");
    push("--no-warnings");
    push("--newline");
    // yt-dlp writes in the Windows code page otherwise: a curly apostrophe is one byte that is not UTF-8, and a character the page lacks is lost.
    push("--encoding");
    push("utf-8");
    push("--progress");
    push("--progress-template");
    push("download:MDPROG %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s");
    // Where the finished file ended up. --print stops yt-dlp from downloading unless told otherwise.
    push("--print");
    push("after_move:FILE:%(filepath)s");
    push("--no-simulate");
    #[cfg(windows)]
    push("--windows-filenames");
    push("-o");
    push(&job.name);
    push(match job.overwrite {
        Overwrite::Skip => "--no-overwrites",
        Overwrite::Replace => "--force-overwrites",
    });
    for arg in job.preset.args() {
        a.push(OsString::from(arg));
    }
    if let (Some(tags), true) = (&job.tags, job.preset.is_audio()) {
        a.extend(tags.args(matches!(job.preset.mode, crate::preset::Mode::Audio { format: crate::preset::AudioFormat::Opus | crate::preset::AudioFormat::Best })).into_iter().map(OsString::from));
    }
    a.push(OsString::from("-P"));
    a.push(job.folder.clone().into_os_string());
    if let Some(dir) = &tools.ffmpeg_dir {
        a.push(OsString::from("--ffmpeg-location"));
        a.push(dir.clone().into_os_string());
    }
    a.extend(crate::deno::runtime_args(tools.deno.as_deref()));
    if let Some(s) = signin {
        a.extend(s.args());
    }
    a.push(OsString::from("--"));
    a.push(OsString::from(&job.url));
    a
}

/// The lines of a process's output. A line that is not valid UTF-8 is read with its bad bytes replaced, never the end of the output: the
/// std line reader gives up at the first one, and with it every line after (the file's path, the reason of a failure).
fn lossy_lines<R: std::io::Read>(from: R) -> impl Iterator<Item = String> {
    BufReader::new(from).split(b'\n').map_while(Result::ok).map(|b| String::from_utf8_lossy(&b).trim_end_matches('\r').to_owned())
}

/// One line of the progress template: `MDPROG <downloaded> <total> <estimated total> <speed> <eta>`, `NA` for what is not known.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let rest = line.strip_prefix("MDPROG ")?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).filter(|n| n.is_finite() && *n >= 0.0);
    let total = num(1).filter(|t| *t > 0.0).or_else(|| num(2).filter(|t| *t > 0.0));
    let fraction = match (num(0), total) {
        (Some(done), Some(total)) => Some((done / total).clamp(0.0, 1.0) as f32),
        _ => None,
    };
    Some(Progress { fraction, speed: num(3).map(|v| v as u64), eta: num(4).map(|v| v as u64) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::seeded;
    use std::{fs, path::Path, sync::atomic::{AtomicUsize, Ordering}, time::{Duration, Instant}};

    #[test]
    fn a_line_that_is_not_utf8_does_not_end_the_output() {
        let bytes: &[u8] = b"MDPROG 1 2\r\nFILE:It\x92s\nFILE:after\n";
        let lines: Vec<String> = lossy_lines(bytes).collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2], "FILE:after");
        assert!(lines[1].starts_with("FILE:It"));
    }

    #[test]
    fn yt_dlp_is_asked_for_utf8() {
        let args = build_args(&job(), &Tools { ytdlp: PathBuf::from("y"), ffmpeg_dir: None, deno: None }, None);
        let at = args.iter().position(|a| a == "--encoding").expect("--encoding");
        assert_eq!(args[at + 1], "utf-8");
    }

    #[test]
    fn a_progress_line_is_read_and_what_is_unknown_stays_unknown() {
        let p = parse_progress("MDPROG 500 1000 NA 2048.5 12").unwrap();
        assert_eq!((p.fraction, p.speed, p.eta), (Some(0.5), Some(2048), Some(12)));
        let p = parse_progress("MDPROG 300 NA 600 NA NA").unwrap();
        assert_eq!((p.fraction, p.speed, p.eta), (Some(0.5), None, None), "the estimate stands in for the total");
        let p = parse_progress("MDPROG NA NA NA NA NA").unwrap();
        assert_eq!(p, Progress::default());
        assert_eq!(parse_progress("[download] 50%").unwrap_or_default(), Progress::default());
        assert!(parse_progress("[download] 50%").is_none());
        assert_eq!(parse_progress("MDPROG 900 600 NA 1 1").unwrap().fraction, Some(1.0), "never above the whole");
    }

    #[test]
    fn the_command_ignores_the_machines_config_and_ends_with_the_link() {
        let job = Job { url: "https://example.test/v".into(), preset: seeded(false).remove(0), folder: PathBuf::from("out dir"), name: crate::destination::template(crate::destination::DEFAULT_PATTERN), overwrite: Overwrite::Skip, tags: None };
        let tools = Tools { ytdlp: PathBuf::from("yt-dlp"), ffmpeg_dir: Some(PathBuf::from("ff")), deno: None };
        let a: Vec<String> = build_args(&job, &tools, None).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(a[0], "--ignore-config");
        assert_eq!(&a[a.len() - 2..], ["--", "https://example.test/v"], "a link is never read as an option");
        assert!(a.windows(2).any(|w| w == ["-P", "out dir"]));
        assert!(a.windows(2).any(|w| w == ["--ffmpeg-location", "ff"]));
        assert!(a.contains(&"--no-overwrites".to_owned()));
        let over = Job { overwrite: Overwrite::Replace, ..job };
        let a: Vec<String> = build_args(&over, &tools, None).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(a.contains(&"--force-overwrites".to_owned()));
    }

    #[test]
    fn yt_dlp_is_given_the_installed_deno_and_nothing_when_there_is_none() {
        let job = Job { url: "https://example.test/v".into(), preset: seeded(false).remove(0), folder: PathBuf::from("out"), name: crate::destination::template(crate::destination::DEFAULT_PATTERN), overwrite: Overwrite::Skip, tags: None };
        let without = Tools { ytdlp: PathBuf::from("yt-dlp"), ffmpeg_dir: None, deno: None };
        let a: Vec<String> = build_args(&job, &without, None).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(!a.contains(&"--js-runtimes".to_owned()));
        let with = Tools { deno: Some(PathBuf::from("dn/deno")), ..without };
        let a: Vec<String> = build_args(&job, &with, None).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(a.windows(2).any(|w| w == ["--js-runtimes", "deno:dn/deno"]));
    }

    // A stand-in for yt-dlp: it fails until a file named `ok` sits beside it, then it reports progress and a finished file.
    fn fake(dir: &Path) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        #[cfg(windows)]
        let (name, body) = ("fake-ytdlp.cmd", "@echo off\r\nif exist \"%~dp0ok\" goto ok\r\necho ERROR: this site changed 1>&2\r\nexit /b 1\r\n:ok\r\necho MDPROG 50 100 NA 10 3\r\necho FILE:%~dp0video.mp4\r\nexit /b 0\r\n");
        #[cfg(not(windows))]
        let (name, body) = ("fake-ytdlp.sh", "#!/bin/sh\nd=$(dirname \"$0\")\nif [ -e \"$d/ok\" ]; then echo 'MDPROG 50 100 NA 10 3'; echo \"FILE:$d/video.mp4\"; exit 0; fi\necho 'ERROR: this site changed' >&2\nexit 1\n");
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yaiol-mdld-run-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn job() -> Job {
        Job { url: "https://example.test/v".into(), preset: seeded(false).remove(0), folder: PathBuf::from("."), name: crate::destination::template(crate::destination::DEFAULT_PATTERN), overwrite: Overwrite::Skip, tags: None }
    }

    fn wait_over(r: &Runner, n: usize) -> Vec<(u64, JobStatus)> {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let snap = r.snapshot();
            if snap.len() == n && snap.iter().all(|(_, s)| !s.is_active()) {
                return snap;
            }
            assert!(Instant::now() < end, "the jobs did not end: {snap:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_download_waits_for_a_lookup_that_is_under_way() {
        let dir = scratch("looking");
        let ytdlp = fake(&dir);
        fs::write(dir.join("ok"), "").unwrap();
        let r = Runner::new(Arc::new(|| Ok(false)));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: None, deno: None }));
        let lookup = Looking::begin();
        let id = r.submit(job());
        std::thread::sleep(Duration::from_millis(700));
        assert!(matches!(r.status(id), Some(JobStatus::Queued)), "{:?}", r.status(id));
        drop(lookup);
        let snap = wait_over(&r, 1);
        assert!(matches!(&snap[0].1, JobStatus::Done(Some(_))), "{:?}", snap[0].1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_job_that_works_is_done_with_the_file_yt_dlp_named() {
        let dir = scratch("ok");
        let ytdlp = fake(&dir);
        fs::write(dir.join("ok"), b"").unwrap();
        let r = Runner::new(Arc::new(|| panic!("nothing failed, nothing to update")));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: None, deno: None }));
        r.submit(job());
        let snap = wait_over(&r, 1);
        assert!(matches!(&snap[0].1, JobStatus::Done(Some(p)) if p.ends_with("video.mp4")), "{:?}", snap[0].1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failure_asks_for_a_newer_yt_dlp_once_and_tries_again_once() {
        let dir = scratch("retry");
        let ytdlp = fake(&dir);
        let calls = Arc::new(AtomicUsize::new(0));
        let (c, d) = (Arc::clone(&calls), dir.clone());
        // The "update" is what makes the stand-in work.
        let r = Runner::new(Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
            fs::write(d.join("ok"), b"").unwrap();
            Ok(true)
        }));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: None, deno: None }));
        r.set_parallel(3);
        for _ in 0..3 {
            r.submit(job());
        }
        let snap = wait_over(&r, 3);
        assert!(snap.iter().all(|(_, s)| matches!(s, JobStatus::Done(_))), "{snap:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "three failures of one batch ask once");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn with_nothing_newer_the_real_error_is_shown_and_nothing_loops() {
        let dir = scratch("none");
        let ytdlp = fake(&dir);
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let r = Runner::new(Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(false)
        }));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: None, deno: None }));
        r.set_parallel(1);
        r.submit(job());
        r.submit(job());
        let snap = wait_over(&r, 2);
        for (_, s) in &snap {
            assert!(matches!(s, JobStatus::Failed(e) if e.contains("this site changed")), "{s:?}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A new batch may ask again.
        r.submit(job());
        wait_over(&r, 3);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(dir);
    }

    // A stand-in for a yt-dlp that asks for a sign-in unless it is given the login of `accept`: every call is written to `calls.txt`.
    fn fake_sign_in(dir: &Path, accept: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let json = r#"{"id":"a","title":"A film","webpage_url":"https://example.test/a"}"#;
        #[cfg(windows)]
        let (name, body) = (
            "fake-signin.cmd",
            format!(
                "@echo off\r\necho %*>>\"%~dp0calls.txt\"\r\necho %* | findstr /c:\"{accept}\" >nul\r\nif %errorlevel%==0 goto ok\r\necho ERROR: [youtube] x: Sign in to confirm your age. Use --cookies-from-browser or --cookies 1>&2\r\nexit /b 1\r\n:ok\r\necho %* | findstr /c:\" -J \" >nul\r\nif %errorlevel%==0 (\r\necho {json}\r\nexit /b 0\r\n)\r\necho FILE:%~dp0video.mp4\r\nexit /b 0\r\n"
            ),
        );
        #[cfg(not(windows))]
        let (name, body) = (
            "fake-signin.sh",
            format!(
                "#!/bin/sh\nd=$(dirname \"$0\")\necho \"$*\" >> \"$d/calls.txt\"\ncase \"$*\" in *\"{accept}\"*)\ncase \"$*\" in *\" -J \"*) echo '{json}'; exit 0;; esac\necho \"FILE:$d/video.mp4\"; exit 0;; esac\necho 'ERROR: [youtube] x: Sign in to confirm your age. Use --cookies-from-browser or --cookies' >&2\nexit 1\n"
            ),
        );
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn calls(dir: &Path) -> Vec<String> {
        fs::read_to_string(dir.join("calls.txt")).unwrap_or_default().lines().map(|l| l.trim().to_owned()).collect()
    }

    fn browsers(list: &[&str]) -> Option<Vec<String>> {
        Some(list.iter().map(|b| (*b).to_owned()).collect())
    }

    #[test]
    fn a_video_that_wants_a_sign_in_borrows_the_login_of_the_browser_that_has_one_and_remembers_it() {
        let dir = scratch("login");
        let ytdlp = fake_sign_in(&dir, "--cookies-from-browser edge");
        // Nothing to update: the updater must not even be asked about a sign-in.
        let r = Runner::new(Arc::new(|| panic!("a sign-in is not a reason to update")));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: None, deno: None }));
        r.set_parallel(1);
        r.set_login(browsers(&["chrome", "edge"]));
        r.submit(job());
        r.submit(job());
        let snap = wait_over(&r, 2);
        assert!(snap.iter().all(|(_, s)| matches!(s, JobStatus::Done(Some(_)))), "{snap:?}");
        let c = calls(&dir);
        assert_eq!(c.len(), 5, "{c:?}");
        assert!(!c[0].contains("--cookies-from-browser"), "the first try has no browser");
        assert!(c[1].contains("--cookies-from-browser chrome") && c[2].contains("--cookies-from-browser edge"));
        assert!(!c[3].contains("--cookies-from-browser"), "the second video also starts plain");
        assert!(c[4].contains("--cookies-from-browser edge"), "then goes straight to the browser that worked: {c:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn with_the_login_switched_off_no_browser_is_touched_and_the_real_error_is_shown() {
        let dir = scratch("login-off");
        let ytdlp = fake_sign_in(&dir, "--cookies-from-browser edge");
        let r = Runner::new(Arc::new(|| panic!("a sign-in is not a reason to update")));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: None, deno: None }));
        r.set_login(None);
        r.submit(job());
        let snap = wait_over(&r, 1);
        assert!(matches!(&snap[0].1, JobStatus::Failed(e) if e.contains("Sign in to confirm")), "{:?}", snap[0].1);
        assert_eq!(calls(&dir).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn when_no_browser_has_a_login_the_real_error_is_shown_after_one_try_each() {
        let dir = scratch("login-none");
        let ytdlp = fake_sign_in(&dir, "--cookies-from-browser vivaldi");
        let r = Runner::new(Arc::new(|| panic!("a sign-in is not a reason to update")));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: None, deno: None }));
        r.set_login(browsers(&["chrome", "edge"]));
        r.submit(job());
        let snap = wait_over(&r, 1);
        assert!(matches!(&snap[0].1, JobStatus::Failed(e) if e.contains("Sign in to confirm")), "{:?}", snap[0].1);
        assert_eq!(calls(&dir).len(), 3, "the plain try, then chrome and edge once each");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_link_that_wants_a_sign_in_is_looked_up_with_a_browsers_login_too() {
        let dir = scratch("login-lookup");
        let ytdlp = fake_sign_in(&dir, "--cookies-from-browser edge");
        let r = Runner::new(Arc::new(|| Ok(false)));
        r.set_login(browsers(&["chrome", "edge"]));
        let items = r.lookup(&ytdlp, "https://example.test/a", &Default::default()).unwrap();
        assert_eq!((items.len(), items[0].title.as_str()), (1, "A film"));
        let off = Runner::new(Arc::new(|| Ok(false)));
        assert!(off.lookup(&ytdlp, "https://example.test/a", &Default::default()).unwrap_err().contains("Sign in to confirm"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_audio_file_is_tagged_by_yt_dlp_without_overwriting_the_comment() {
        let tags = Tags { album: Some(r"A: 100% b".to_owned()), track: Some((3, 12)) };
        let tools = Tools { ytdlp: PathBuf::from("yt-dlp"), ffmpeg_dir: None, deno: None };
        let words = |p: Preset, t: Option<Tags>| -> Vec<String> { build_args(&Job { preset: p, tags: t, ..job() }, &tools, None).iter().map(|s| s.to_string_lossy().into_owned()).collect() };
        let mp3 = || seeded(true).into_iter().find(|p| p.id == "audio-mp3").unwrap();
        let audio = words(mp3(), Some(tags));
        for want in ["--embed-metadata", "--embed-thumbnail", "post_process:%(webpage_url)s:%(meta_purl)s", "post_process::(?P<webpage_url>)", r"A\: 100%% b:%(album)s", "3/12:%(meta_track)s"] {
            assert!(audio.contains(&want.to_owned()), "{want}: {audio:?}");
        }
        let copy = audio.iter().position(|x| x == "post_process:%(webpage_url)s:%(meta_purl)s").unwrap();
        let clear = audio.iter().position(|x| x == "post_process::(?P<webpage_url>)").unwrap();
        assert!(copy < clear, "the address is copied before it is emptied");
        assert!(!audio.iter().any(|x| x.contains("description")), "the site's description is yt-dlp's own business");
        let opus = seeded(true).into_iter().find(|p| p.id == "audio-opus").unwrap();
        assert!(words(opus, Some(Tags::default())).contains(&"post_process::(?P<description>)".to_owned()), "an opus keeps its comment in DESCRIPTION");
        let best = seeded(true).into_iter().find(|p| p.id == "audio-best").unwrap();
        assert!(words(best, Some(Tags::default())).contains(&"post_process::(?P<description>)".to_owned()), "the site's own format may be an opus");
        assert!(!audio.iter().any(|x| x.contains("movflags")), "the option that made the files unreadable is gone");
        assert!(!words(seeded(false).remove(0), Some(Tags::default())).iter().any(|x| x.contains("metadata")), "a video is left alone");
        assert!(!words(mp3(), None).iter().any(|x| x.contains("metadata")), "tagging off leaves the file as the site made it");
    }

    #[test]
    fn the_sign_in_is_named_just_before_the_link_and_only_when_asked() {
        let tools = Tools { ytdlp: PathBuf::from("yt-dlp"), ffmpeg_dir: None, deno: None };
        let words = |s: Option<&Signin>| -> Vec<String> { build_args(&job(), &tools, s).iter().map(|s| s.to_string_lossy().into_owned()).collect() };
        let a = words(Some(&Signin::Browser("firefox".to_owned())));
        assert_eq!(&a[a.len() - 4..], ["--cookies-from-browser", "firefox", "--", "https://example.test/v"]);
        let a = words(Some(&Signin::File(PathBuf::from("c.txt"))));
        assert_eq!(&a[a.len() - 4..], ["--cookies", "c.txt", "--", "https://example.test/v"]);
        assert!(!words(None).iter().any(|x| x.contains("cookies")));
    }

    #[test]
    fn a_cookies_file_is_tried_before_any_browser_even_with_the_browsers_off() {
        let dir = scratch("login-file");
        let file = dir.join("cookies.txt");
        let ytdlp = fake_sign_in(&dir, "--cookies ");
        fs::write(&file, "# Netscape HTTP Cookie File\n").unwrap();
        let r = Runner::new(Arc::new(|| panic!("a sign-in is not a reason to update")));
        r.set_tools(Some(Tools { ytdlp: ytdlp.clone(), ffmpeg_dir: None, deno: None }));
        r.set_login(None);
        r.set_cookies_file(Some(file.clone()));
        r.submit(job());
        let snap = wait_over(&r, 1);
        assert!(matches!(&snap[0].1, JobStatus::Done(Some(_))), "{:?}", snap[0].1);
        let c = calls(&dir);
        assert_eq!(c.len(), 2, "{c:?}");
        assert!(c[1].contains("--cookies ") && !c[1].contains("--cookies-from-browser"));
        // A file that is gone is not tried.
        fs::remove_file(&file).unwrap();
        r.submit(job());
        wait_over(&r, 2);
        assert_eq!(calls(&dir).len(), 3, "only the plain try: {:?}", calls(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    /// Needs the network, the installed programs and their folders in the environment (`MDLD_YTDLP`, `MDLD_FFMPEG_DIR`): downloads a
    /// 19 second clip through the real yt-dlp as video and as sound, and checks the files. Run with `--ignored`.
    #[test]
    #[ignore]
    fn a_real_clip_is_downloaded_as_video_and_as_sound() {
        let ytdlp = PathBuf::from(std::env::var("MDLD_YTDLP").expect("MDLD_YTDLP"));
        let ffmpeg_dir = PathBuf::from(std::env::var("MDLD_FFMPEG_DIR").expect("MDLD_FFMPEG_DIR"));
        let out = scratch("real");
        let r = Runner::new(Arc::new(|| Ok(false)));
        r.set_tools(Some(Tools { ytdlp, ffmpeg_dir: Some(ffmpeg_dir), deno: None }));
        let presets: Vec<_> = seeded(false).into_iter().chain(seeded(true)).collect();
        for id in ["video-mp4-720", "audio-mp3"] {
            let preset = presets.iter().find(|p| p.id == id).unwrap().clone();
            r.submit(Job { url: "https://www.youtube.com/watch?v=jNQXAC9IVRw".into(), preset, folder: out.clone(), name: crate::destination::template(crate::destination::DEFAULT_PATTERN), overwrite: Overwrite::Skip, tags: None });
        }
        let snap = wait_over(&r, 2);
        for (_, s) in &snap {
            assert!(matches!(s, JobStatus::Done(Some(p)) if p.is_file()), "{s:?}");
        }
        let names: Vec<String> = fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        eprintln!("{names:?}");
        assert!(names.iter().any(|n| n.ends_with(".mp4")) && names.iter().any(|n| n.ends_with(".mp3")), "{names:?}");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_job_without_the_programs_fails_and_a_queued_job_can_be_cancelled() {
        let r = Runner::new(Arc::new(|| Ok(false)));
        let id = r.submit(job());
        let snap = wait_over(&r, 1);
        assert!(matches!(&snap[0].1, JobStatus::Failed(e) if e.contains("not installed")), "{:?}", snap[0].1);
        r.forget(id);
        assert!(r.snapshot().is_empty());
        assert_eq!(r.status(id), None);
    }
}
