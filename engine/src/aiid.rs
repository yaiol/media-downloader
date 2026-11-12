//! The last step of a tagged audio download: a Suno id goes into the `YAIOL_AIID` tag, and the page address into `COMMENT`.
//!
//! A SoundCloud file made with Suno carries `made with suno; created=...; id=<uuid>` in its own `COMMENT`, and the owner's workflow keeps that
//! id in a dedicated `YAIOL_AIID=suno:<uuid>` field instead (the comment then holds the page address, as with any other file). yt-dlp cannot do the move: it overwrites the comment
//! with the page address while it tags, and it never reads what the file already holds. So yt-dlp is told to leave the comment alone (see
//! `runner::Tags::args`) and this runs on the finished file:
//!
//! - the comment holds a Suno id: `YAIOL_AIID` is written, read back, and only then is the comment replaced by the page address;
//! - it does not: the comment becomes the page address, which is what yt-dlp would have written itself.
//!
//! One tag library (`lofty`) writes all three formats the audio presets make, and each one's own custom-field form: a `TXXX` frame in an
//! mp3, the free-form `----` atom in an m4a (the form Mp3tag reads; ffmpeg cannot write it), a Vorbis comment in an opus.

use lofty::{
    config::{ParseOptions, WriteOptions},
    file::AudioFile,
    id3::v2::{ExtendedTextFrame, Frame, FrameId, Id3v2Tag, Id3v2Version},
    mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4File},
    mpeg::MpegFile,
    ogg::{OpusFile, tag::VorbisComments},
    tag::Accessor,
    TextEncoding,
};
use std::path::Path;

/// The tag's name. Never changes: the workspace's library and its tools all look for exactly this.
const TAG: &str = "YAIOL_AIID";

/// What was done to a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The Suno id moved into `YAIOL_AIID` and the comment became the page address.
    IdMoved(String),
    /// No Suno id: the comment is the page address.
    CommentSet,
    /// The file already has a `YAIOL_AIID`, or is not a format handled here: left exactly as it is.
    Untouched,
}

/// The first Suno song id in a comment: `id=` followed by a UUID (`made with suno; created=...; id=95319479-...`).
pub fn suno_id(comment: &str) -> Option<String> {
    let bytes = comment.as_bytes();
    let mut from = 0;
    while let Some(at) = comment[from..].find("id=") {
        let start = from + at;
        from = start + 3;
        // `id=` must start a word: `uuid=` or `valid=` are something else.
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        if let Some(candidate) = comment.get(from..from + 36).filter(|c| is_uuid(c)) {
            return Some(candidate.to_lowercase());
        }
    }
    None
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// The three tag types, seen through the few fields this step needs.
trait Fields {
    fn comment_text(&self) -> Option<String>;
    fn set_comment_text(&mut self, text: &str);
    fn clear_comment(&mut self);
    fn aiid(&self) -> Option<String>;
    fn set_aiid(&mut self, value: &str);
}

impl Fields for Id3v2Tag {
    // A comment is a `COMM` frame of any description, or a custom `comment` text frame (what ffmpeg writes for the field): both are read,
    // and both are removed when the comment is replaced or emptied, so no second copy of an id is left behind.
    fn comment_text(&self) -> Option<String> {
        self.iter().find_map(|f| match f {
            Frame::Comment(c) if !c.content.is_empty() => Some(c.content.to_string()),
            Frame::UserText(t) if t.description.eq_ignore_ascii_case("comment") && !t.content.is_empty() => Some(t.content.to_string()),
            _ => None,
        })
    }
    fn set_comment_text(&mut self, text: &str) {
        self.clear_comment();
        self.set_comment(text.to_owned());
    }
    fn clear_comment(&mut self) {
        let _ = self.remove(&FrameId::Valid("COMM".into())).count();
        let spellings: Vec<String> = self
            .iter()
            .filter_map(|f| match f {
                Frame::UserText(t) if t.description.eq_ignore_ascii_case("comment") => Some(t.description.to_string()),
                _ => None,
            })
            .collect();
        for s in spellings {
            let _ = self.remove_user_text(&s);
        }
    }
    fn aiid(&self) -> Option<String> {
        self.iter().find_map(|f| match f {
            Frame::UserText(t) if t.description.eq_ignore_ascii_case(TAG) => Some(t.content.to_string()),
            _ => None,
        })
    }
    fn set_aiid(&mut self, value: &str) {
        let _ = self.remove_user_text(TAG);
        // UTF-16 is what ID3v2.3, which ffmpeg writes by default, can hold; v2.4 files read it all the same.
        self.insert(Frame::UserText(ExtendedTextFrame::new(TextEncoding::UTF16, TAG.to_owned(), value.to_owned())));
    }
}

fn freeform() -> AtomIdent<'static> {
    AtomIdent::Freeform { mean: "com.apple.iTunes".into(), name: TAG.into() }
}

impl Fields for Ilst {
    fn comment_text(&self) -> Option<String> {
        self.comment().map(|c| c.into_owned())
    }
    fn set_comment_text(&mut self, text: &str) {
        self.set_comment(text.to_owned());
    }
    fn clear_comment(&mut self) {
        self.remove_comment();
    }
    fn aiid(&self) -> Option<String> {
        self.get(&freeform()).and_then(|a| a.data().find_map(|d| if let AtomData::UTF8(s) = d { Some(s.clone()) } else { None }))
    }
    fn set_aiid(&mut self, value: &str) {
        let _ = self.remove(&freeform());
        self.insert(Atom::new(freeform(), AtomData::UTF8(value.to_owned())));
    }
}

/// Every spelling of `name` the tag holds: Vorbis names are case-insensitive and ffmpeg writes some of them in lower case.
fn spellings(vc: &VorbisComments, name: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for (k, _) in vc.items() {
        if k.eq_ignore_ascii_case(name) && !seen.iter().any(|s| s == k) {
            seen.push(k.to_owned());
        }
    }
    seen
}

fn forget(vc: &mut VorbisComments, name: &str) {
    for k in spellings(vc, name) {
        let _ = vc.remove(&k).count();
    }
}

impl Fields for VorbisComments {
    fn comment_text(&self) -> Option<String> {
        // ffmpeg writes the field `comment` of an opus as `DESCRIPTION` (Vorbis has no comment field); a tag editor writes `COMMENT`.
        let find = |name: &str| self.items().find(|(k, v)| k.eq_ignore_ascii_case(name) && !v.is_empty()).map(|(_, v)| v.to_owned());
        find("COMMENT").or_else(|| find("DESCRIPTION"))
    }
    fn set_comment_text(&mut self, text: &str) {
        self.clear_comment();
        self.push("DESCRIPTION".to_owned(), text.to_owned());
    }
    fn clear_comment(&mut self) {
        forget(self, "comment");
        forget(self, "description");
    }
    fn aiid(&self) -> Option<String> {
        self.items().find(|(k, _)| k.eq_ignore_ascii_case(TAG)).map(|(_, v)| v.to_owned())
    }
    fn set_aiid(&mut self, value: &str) {
        forget(self, TAG);
        self.push(TAG.to_owned(), value.to_owned());
    }
}

/// Open `path`, hand its tag to `edit`, save when `save` is true. `None` when the file is not a format handled here.
fn with_tags<R>(path: &Path, save: bool, edit: impl FnOnce(&mut dyn Fields) -> R) -> Result<Option<R>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let parse = ParseOptions::new();
    let io = |e: std::io::Error| format!("cannot open the file: {e}");
    let bad = |e: &dyn std::fmt::Display| format!("the tags could not be handled: {e}");
    let open = || std::fs::File::open(path).map_err(io);
    match ext.as_str() {
        "mp3" => {
            let mut file = MpegFile::read_from(&mut open()?, parse).map_err(|e| bad(&e))?;
            let mut tag = file.id3v2_mut().map(std::mem::take).unwrap_or_default();
            let v23 = tag.original_version() == Id3v2Version::V3;
            let r = edit(&mut tag);
            if save {
                file.set_id3v2(tag);
                file.save_to_path(path, WriteOptions::new().use_id3v23(v23)).map_err(|e| bad(&e))?;
            }
            Ok(Some(r))
        }
        "m4a" | "mp4" => {
            let mut file = Mp4File::read_from(&mut open()?, parse).map_err(|e| bad(&e))?;
            let mut ilst = file.ilst_mut().map(std::mem::take).unwrap_or_default();
            let r = edit(&mut ilst);
            if save {
                file.set_ilst(ilst);
                file.save_to_path(path, WriteOptions::new()).map_err(|e| bad(&e))?;
            }
            Ok(Some(r))
        }
        "opus" => {
            let mut file = OpusFile::read_from(&mut open()?, parse).map_err(|e| bad(&e))?;
            let r = edit(file.vorbis_comments_mut());
            if save {
                file.save_to_path(path, WriteOptions::new()).map_err(|e| bad(&e))?;
            }
            Ok(Some(r))
        }
        _ => Ok(None),
    }
}

/// Finish one downloaded audio file. `page_url` is the address of the page it came from.
///
/// The order is the workspace's own rule for this tag: write `YAIOL_AIID`, read it back from the file, and only then replace the comment, so
/// a failed write never costs the id.
pub fn finish(path: &Path, page_url: &str) -> Result<Outcome, String> {
    let Some((already, comment)) = with_tags(path, false, |t| (t.aiid().is_some(), t.comment_text()))? else {
        return Ok(Outcome::Untouched);
    };
    // A file that already has its `YAIOL_AIID` was handled before (a download that found the file there): its comment was settled then.
    if already {
        return Ok(Outcome::Untouched);
    }
    match comment.as_deref().and_then(suno_id) {
        Some(id) => {
            let value = format!("suno:{id}");
            with_tags(path, true, |t| t.set_aiid(&value))?;
            let back = with_tags(path, false, |t| t.aiid())?.flatten();
            if back.as_deref() != Some(value.as_str()) {
                return Err(format!("YAIOL_AIID could not be read back from the file (wrote {value}); the comment was kept"));
            }
            with_tags(path, true, |t| t.set_comment_text(page_url))?;
            Ok(Outcome::IdMoved(id))
        }
        None => {
            with_tags(path, true, |t| t.set_comment_text(page_url))?;
            Ok(Outcome::CommentSet)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suno_id_is_found_in_the_comment_and_nothing_else_is() {
        let c = "made with suno; created=2026-09-01T10:00:00.000Z; id=95319479-AAAA-4bcd-8e12-0123456789ab";
        assert_eq!(suno_id(c).as_deref(), Some("95319479-aaaa-4bcd-8e12-0123456789ab"), "lower case, as the library keeps it");
        assert_eq!(suno_id("uuid=95319479-aaaa-4bcd-8e12-0123456789ab"), None, "`id=` must start a word");
        assert_eq!(suno_id("id=not-a-uuid"), None);
        assert_eq!(suno_id("https://soundcloud.com/a/b"), None);
        assert_eq!(suno_id(""), None);
    }

    /// Needs `MDLD_FFMPEG_DIR` (a folder holding ffmpeg), like the real-clip test: makes one real file per format.
    #[test]
    #[ignore = "needs ffmpeg"]
    fn the_id_moves_in_every_format_and_any_other_comment_becomes_the_address() {
        let dir = std::env::temp_dir().join(format!("mdld-aiid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = crate::ffmpeg::locate(Some(Path::new(&std::env::var("MDLD_FFMPEG_DIR").expect("MDLD_FFMPEG_DIR")))).expect("ffmpeg in MDLD_FFMPEG_DIR");
        let suno = "made with suno; created=2026-09-01T10:00:00.000Z; id=95319479-aaaa-4bcd-8e12-0123456789ab";
        let make = |name: &str, codec: &[&str], comment: &str| -> std::path::PathBuf {
            let out = dir.join(name);
            let ok = crate::ffmpeg::command(&ffmpeg)
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-t", "1"])
                .args(codec)
                .args(["-metadata", "title=T", "-metadata", &format!("comment={comment}")])
                .arg(&out)
                .status()
                .unwrap()
                .success();
            assert!(ok, "ffmpeg made {name}");
            out
        };
        for (name, codec) in [("a.mp3", ["-c:a", "libmp3lame"]), ("a.m4a", ["-c:a", "aac"]), ("a.opus", ["-c:a", "libopus"])] {
            let f = make(name, &codec, suno);
            assert_eq!(finish(&f, "https://example.test/p").unwrap(), Outcome::IdMoved("95319479-aaaa-4bcd-8e12-0123456789ab".to_owned()), "{name}");
            let (aiid, comment, title) = with_tags(&f, false, |t| (t.aiid(), t.comment_text(), ())).unwrap().map(|(a, c, _)| (a, c, ())).unwrap();
            assert_eq!(aiid.as_deref(), Some("suno:95319479-aaaa-4bcd-8e12-0123456789ab"), "{name}");
            assert_eq!(comment.as_deref(), Some("https://example.test/p"), "{name}: the comment is the page address");
            let _ = title;
            assert_eq!(finish(&f, "https://example.test/p").unwrap(), Outcome::Untouched, "{name}: a second run leaves it alone");
            let g = make(&format!("b{name}"), &codec, "just words");
            assert_eq!(finish(&g, "https://example.test/p").unwrap(), Outcome::CommentSet, "{name}");
            let comment = with_tags(&g, false, |t| t.comment_text()).unwrap().unwrap();
            assert_eq!(comment.as_deref(), Some("https://example.test/p"), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
