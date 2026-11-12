//! The little of ffmpeg this app needs: where its program is, and whether it is a build we may use. Media Downloader never runs
//! ffmpeg itself - it hands the folder to yt-dlp, which merges the video and the sound with it - but it installs the one yaiol
//! approved (`ffmpeg_install`), exactly as the converters do, and so must recognise that install.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub fn executable_name() -> &'static str {
    if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }
}

/// The ffmpeg to use: the one the app installed into `dir`, and no other. PATH is never
/// searched, and a program in `dir` that no install recorded (one put there by hand) is not a build we know.
pub fn locate(dir: Option<&Path>) -> Option<PathBuf> {
    let dir = dir?;
    crate::ffmpeg_install::installed_version(dir)?;
    Some(dir.join(executable_name()))
}

/// A `Command` that does not flash a console window on Windows.
pub fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// What ffmpeg's `-version` banner says about the build: it may not be a non-free one (we neither
/// download nor knowingly run those). `None` when the program does not answer.
pub fn is_nonfree(ffmpeg: &Path) -> Option<bool> {
    let out = command(ffmpeg).arg("-version").stdin(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).contains("--enable-nonfree"))
}
