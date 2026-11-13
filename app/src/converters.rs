//! The two converters of the same family, when they are installed: a finished download can be opened in one of them.
//!
//! An installed converter is found through the entry its installer writes for the system's list of installed programs (the folder it
//! was installed in and its version). Only a version that opens a file named at its start counts: an older one would open with nothing
//! in it. Windows only: elsewhere nothing is looked for and the answer is "not installed".

use std::path::{Path, PathBuf};

/// One of the two converters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Converter {
    /// Its program id: the name of its executable, and the middle of its installer's entry.
    id: &'static str,
    /// Its name, as its own window shows it.
    pub name: &'static str,
    /// The first version that opens a file named at its start.
    since: &'static str,
}

/// The converter a finished video is opened in.
pub const VIDEO: Converter = Converter { id: "ffmpeg-video-studio", name: "FFmpeg Video Studio", since: "2.0.2" };
/// The converter a finished audio file is opened in.
pub const AUDIO: Converter = Converter { id: "ffmpeg-audio-studio", name: "FFmpeg Audio Studio", since: "2.0.2" };

impl Converter {
    /// The installed program, when there is one that can open a file.
    pub fn installed(&self) -> Option<PathBuf> {
        let (folder, version) = entry(self.id)?;
        self.program(Path::new(&folder), &version)
    }

    /// The program in `folder` when `version` is recent enough and the program is there.
    fn program(&self, folder: &Path, version: &str) -> Option<PathBuf> {
        if engine::compare_versions(version, self.since).is_lt() {
            return None;
        }
        let exe = folder.join(format!("{}.exe", self.id));
        exe.is_file().then_some(exe)
    }
}

/// Start `program` on `file`. A failure is logged, not shown: the button did nothing, and the log says why.
pub fn open(program: &Path, file: &Path) {
    if let Err(e) = std::process::Command::new(program).arg(file).spawn() {
        engine::log::error("open in converter", &format!("{e}\nprogram: {}\nfile: {}", program.display(), file.display()));
    }
}

/// The folder and the version the installer of `id` recorded: for this user first, then for the whole machine.
#[cfg(windows)]
fn entry(id: &str) -> Option<(String, String)> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let key = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\com.yaiol.{id}.app");
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE].into_iter().find_map(|root| Some((text(root, &key, "InstallLocation")?, text(root, &key, "DisplayVersion")?)))
}

#[cfg(not(windows))]
fn entry(_id: &str) -> Option<(String, String)> {
    None
}

/// One text value of the registry, in whichever of the 64-bit and 32-bit views holds it.
#[cfg(windows)]
fn text(root: windows_sys::Win32::System::Registry::HKEY, key: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key, name) = (wide(key), wide(name));
    [RRF_SUBKEY_WOW6464KEY, RRF_SUBKEY_WOW6432KEY].into_iter().find_map(|view| {
        let mut data = [0u16; 1024];
        let mut size = std::mem::size_of_val(&data) as u32;
        // SAFETY: a read of one string into a buffer whose size in bytes is given; the names are NUL-terminated.
        let status = unsafe { RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ | view, std::ptr::null_mut(), data.as_mut_ptr().cast(), &mut size) };
        if status != 0 {
            return None;
        }
        let len = data.iter().position(|c| *c == 0).unwrap_or(data.len());
        let value = String::from_utf16_lossy(&data[..len]);
        (!value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_version_that_opens_a_file_counts_and_only_when_its_program_is_there() {
        let folder = std::env::temp_dir().join(format!("mdld-converters-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(VIDEO.program(&folder, "2.0.2"), None, "no program in the folder");
        let exe = folder.join("ffmpeg-video-studio.exe");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(VIDEO.program(&folder, "2.0.1"), None, "an older version would open empty");
        assert_eq!(VIDEO.program(&folder, "2.0.2"), Some(exe.clone()));
        assert_eq!(VIDEO.program(&folder, "2.1.0"), Some(exe));
        assert_eq!(AUDIO.program(&folder, "2.0.2"), None, "the other converter is not in this folder");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
