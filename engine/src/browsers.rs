//! Which browsers this PC has, by the names yt-dlp knows them as (`--cookies-from-browser <name>`).
//!
//! Used when a site wants a signed-in visitor (an age-restricted video, a members-only one): yt-dlp can read the login from a browser the
//! user is already signed in with. The order is the order they are tried in. Firefox first, because it is the one whose cookies yt-dlp can
//! read while the browser is open and on every version; Chrome, Edge and Brave encrypt theirs in a way that often fails on Windows.
//!
//! Only the presence of the browser's profile folder is looked at. Nothing is read from it here; yt-dlp does that, in the one run that needs it.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Where a sign-in comes from: a browser the PC has, or a cookies file the user exported and picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Signin {
    Browser(String),
    File(PathBuf),
}

impl Signin {
    /// The yt-dlp options that carry it.
    pub fn args(&self) -> [OsString; 2] {
        match self {
            Signin::Browser(b) => [OsString::from("--cookies-from-browser"), OsString::from(b)],
            Signin::File(p) => [OsString::from("--cookies"), p.clone().into_os_string()],
        }
    }
}

/// The browsers worth trying, in order, as yt-dlp names them.
pub const ORDER: [&str; 6] = ["firefox", "edge", "chrome", "brave", "vivaldi", "opera"];

/// Where a browser keeps its profiles on this system, or `None` when this system has no such browser to look for.
fn profile_dir(name: &str, env: &impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(windows) {
        let local = env("LOCALAPPDATA");
        let roaming = env("APPDATA");
        match name {
            "firefox" => Some(roaming?.join("Mozilla").join("Firefox").join("Profiles")),
            "edge" => Some(local?.join("Microsoft").join("Edge").join("User Data")),
            "chrome" => Some(local?.join("Google").join("Chrome").join("User Data")),
            "brave" => Some(local?.join("BraveSoftware").join("Brave-Browser").join("User Data")),
            "vivaldi" => Some(local?.join("Vivaldi").join("User Data")),
            "opera" => Some(roaming?.join("Opera Software").join("Opera Stable")),
            _ => None,
        }
    } else if cfg!(target_os = "macos") {
        let support = env("HOME")?.join("Library").join("Application Support");
        match name {
            "firefox" => Some(support.join("Firefox").join("Profiles")),
            "edge" => Some(support.join("Microsoft Edge")),
            "chrome" => Some(support.join("Google").join("Chrome")),
            "brave" => Some(support.join("BraveSoftware").join("Brave-Browser")),
            "vivaldi" => Some(support.join("Vivaldi")),
            "opera" => Some(support.join("com.operasoftware.Opera")),
            _ => None,
        }
    } else {
        let home = env("HOME")?;
        match name {
            "firefox" => Some(home.join(".mozilla").join("firefox")),
            "edge" => Some(home.join(".config").join("microsoft-edge")),
            "chrome" => Some(home.join(".config").join("google-chrome")),
            "brave" => Some(home.join(".config").join("BraveSoftware").join("Brave-Browser")),
            "vivaldi" => Some(home.join(".config").join("vivaldi")),
            "opera" => Some(home.join(".config").join("opera")),
            _ => None,
        }
    }
}

fn from_environment(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

/// The browsers of [`ORDER`] whose profile folder exists.
pub fn installed() -> Vec<String> {
    installed_with(&from_environment, &|p: &Path| p.is_dir())
}

fn installed_with(env: &impl Fn(&str) -> Option<PathBuf>, is_dir: &impl Fn(&Path) -> bool) -> Vec<String> {
    ORDER.iter().filter(|name| profile_dir(name, env).is_some_and(|d| is_dir(&d))).map(|name| (*name).to_owned()).collect()
}

/// Does this failure mean "the site wants you signed in"? yt-dlp words it several ways and each names either the sign-in or its own
/// `--cookies` options.
pub fn needs_login(reason: &str) -> bool {
    let r = reason.to_lowercase();
    r.contains("sign in") || r.contains("--cookies") || r.contains("log in") || r.contains("login required")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browsers_found_are_the_ones_with_a_profile_folder_in_the_order_they_are_tried() {
        let env = |key: &str| Some(PathBuf::from(format!("/home/{key}")));
        // Pretend only Chrome's and Firefox's folders exist.
        let chrome = profile_dir("chrome", &env).unwrap();
        let firefox = profile_dir("firefox", &env).unwrap();
        let found = installed_with(&env, &|p: &Path| p == chrome || p == firefox);
        assert_eq!(found, ["firefox", "chrome"], "Firefox first even though Chrome comes earlier on disk");
        assert!(installed_with(&env, &|_: &Path| false).is_empty());
    }

    #[test]
    fn a_missing_environment_variable_means_the_browser_is_not_found() {
        assert!(installed_with(&|_: &str| None, &|_: &Path| true).is_empty());
    }

    #[test]
    fn the_sign_in_wordings_of_yt_dlp_are_recognised_and_other_failures_are_not() {
        assert!(needs_login("[youtube] Gt0WvHLxZTA: Sign in to confirm your age. This video may be inappropriate for some users. Use --cookies-from-browser or --cookies for the authentication."));
        assert!(needs_login("Sign in to confirm you're not a bot"));
        assert!(needs_login("Private video. Sign in if you've been granted access to this video"));
        assert!(!needs_login("Video unavailable"));
        assert!(!needs_login("HTTP Error 403: Forbidden"));
    }
}
