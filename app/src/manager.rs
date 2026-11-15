//! The preset editor, a page of the Settings view: the list on the left (drag a row to reorder, new, duplicate, delete), the options of
//! the selected preset on the right. Every change is stored at once, there is no Save button. The saved order is the order of the
//! panel's drop-down too.
//!
//! ⚠ CLAUDE: NO HARDCODED UI STRINGS. File types (MP4, MKV, WebM, MP3, M4A, Opus) and heights (1080p) are names and stay literal;
//! everything else goes through the t() lookup with a literal key at the call site.

use crate::{
    catalog::UisNum,
    i18n,
    settings::{self, Settings},
};
use dioxus::{html::input_data::MouseButton, prelude::*};
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdCopy, LdFolderOpen, LdPlus, LdTrash2},
};
use engine::{
    destination::{self, Destination},
    preset::{self, AudioFormat, Container, Mode, Preset},
    traffic::{self, Traffic},
};

/// The keys of the Settings tabs of the video presets and of the audio presets (as in Video Studio).
pub const PRESETS: &str = "presets";
pub const AUDIO: &str = "audio";
/// The key of the Settings tab of the destinations.
pub const DESTINATIONS: &str = "destinations";

/// Put the dragged entry into the slot of the one it was dragged over. The order of the saved list IS the order everywhere (this list
/// and the panel's drop-down), so this is the whole feature: the editor only decides when to call it.
fn reorder<T>(items: &mut Vec<T>, id: impl Fn(&T) -> &str, dragged: &str, target: &str) {
    let from = items.iter().position(|x| id(x) == dragged);
    let to = items.iter().position(|x| id(x) == target);
    if let (Some(from), Some(to)) = (from, to) {
        if from != to {
            let item = items.remove(from);
            items.insert(to, item);
        }
    }
}

fn edit_preset(mut settings: Signal<Settings>, audio: bool, id: &str, edit: impl FnOnce(&mut Preset)) {
    let mut s = settings.write();
    if let Some(p) = s.list_mut(audio).iter_mut().find(|p| p.id == id) {
        edit(p);
    }
}

#[component]
pub fn PresetEditor(settings: Signal<Settings>, audio: bool) -> Element {
    let mut selected = use_signal(|| settings.read().picked_id(audio).to_owned());
    let mut dragging = use_signal(|| Option::<String>::None);

    let lang = settings.read().lang.clone();
    let t = |key: &str| i18n::t(&lang, key);

    let presets = engine::preset::repaired(settings.read().list(audio).clone(), audio);
    let current = presets.iter().find(|p| p.id == selected()).unwrap_or(&presets[0]).clone();
    let cur_id = current.id.clone();
    let can_delete = presets.len() > 1;

    let s_new = t("btnMngNew");
    let s_dup = t("btnMngDuplicate");
    let s_del = t("btnMngDelete");
    let s_reorder = t("msgMngReorderHint");
    let s_name = t("lblPrsName");
    let s_height = t("lblPrsMaxHeight");
    let s_best = t("lblPrsHeightBest");
    let s_container = t("lblPrsContainer");
    let s_format = t("lblPrsAudioFormat");
    let s_format_best = t("lblPrsFormatBest");

    let id_for_name = cur_id.clone();
    let id_for_height = cur_id.clone();
    let id_for_container = cur_id.clone();
    let id_for_format = cur_id.clone();
    let id_for_dup = cur_id.clone();
    let id_for_del = cur_id.clone();

    // The heights offered, plus the one this preset already holds when it is not among them (a file edited by hand).
    let (max_height, container, format) = match &current.mode {
        Mode::Video { max_height, container } => (*max_height, Some(*container), None),
        Mode::Audio { format } => (None, None, Some(*format)),
    };
    let mut heights: Vec<u32> = preset::HEIGHTS.to_vec();
    if let Some(h) = max_height {
        if !heights.contains(&h) {
            heights.push(h);
            heights.sort_unstable_by(|a, b| b.cmp(a));
        }
    }

    rsx! {
        div { class: "manager",
            div { class: "manager-list uis-lvw-reorder",
                div { class: "manager-actions",
                    button {
                        class: "uis-btn ico",
                        title: "{s_new}",
                        "aria-label": "{s_new}",
                        onclick: move |_| {
                            let p = Preset::new(&s_new, audio);
                            selected.set(p.id.clone());
                            settings.write().list_mut(audio).push(p);
                        },
                        Icon { width: 14, height: 14, icon: LdPlus }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_dup}",
                        "aria-label": "{s_dup}",
                        onclick: move |_| {
                            let src = settings.read().list(audio).iter().find(|p| p.id == id_for_dup).cloned();
                            if let Some(mut p) = src {
                                p.id = preset::new_id();
                                p.name = format!("{} 2", p.name);
                                selected.set(p.id.clone());
                                settings.write().list_mut(audio).push(p);
                            }
                        },
                        Icon { width: 14, height: 14, icon: LdCopy }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_del}",
                        "aria-label": "{s_del}",
                        disabled: !can_delete,
                        onclick: move |_| {
                            settings.write().list_mut(audio).retain(|p| p.id != id_for_del);
                            let first = engine::preset::repaired(settings.read().list(audio).clone(), audio)[0].id.clone();
                            selected.set(first.clone());
                            // The one picked in the panel may be the one just deleted.
                            if !settings.read().list(audio).iter().any(|p| p.id == settings.read().picked_id(audio)) {
                                settings.write().set_picked(audio, first);
                            }
                        },
                        Icon { width: 14, height: 14, icon: LdTrash2 }
                    }
                }
                div { class: "uis-lvw-scroll",
                    for p in presets.iter() {
                        {
                            let id = p.id.clone();
                            let id_down = p.id.clone();
                            let id_over = p.id.clone();
                            let active = p.id == cur_id;
                            let moving = dragging().as_deref() == Some(p.id.as_str());
                            rsx! {
                                button {
                                    key: "{p.id}",
                                    class: match (active, moving) {
                                        (_, true) => "uis-lvw-item reordering",
                                        (true, _) => "uis-lvw-item active",
                                        _ => "uis-lvw-item",
                                    },
                                    onpointerdown: move |e| {
                                        if e.trigger_button() == Some(MouseButton::Primary) {
                                            dragging.set(Some(id_down.clone()));
                                        }
                                    },
                                    onpointerenter: move |e| {
                                        let Some(from) = dragging() else { return };
                                        if !e.held_buttons().contains(MouseButton::Primary) {
                                            dragging.set(None);
                                            return;
                                        }
                                        reorder(settings.write().list_mut(audio), |p| p.id.as_str(), &from, &id_over);
                                    },
                                    onpointerup: move |_| dragging.set(None),
                                    onclick: move |_| {
                                        selected.set(id.clone());
                                        settings.write().set_picked(audio, id.clone());
                                    },
                                    div { class: "uis-lvw-item-body",
                                        div { class: "uis-lvw-item-name", "{p.name}" }
                                    }
                                    if !p.badge().is_empty() {
                                        span { class: "uis-lvw-item-badge", "{p.badge()}" }
                                    }
                                }
                            }
                        }
                    }
                }
                p { class: "uis-hnt", "{s_reorder}" }
            }
            div { class: "manager-form",
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_name}" }
                    input {
                        class: "uis-txt",
                        value: "{current.name}",
                        oninput: move |e| {
                            let v = e.value();
                            edit_preset(settings, audio, &id_for_name, |p| p.name = v);
                        },
                    }
                }
                if let Some(container) = container {
                    div { class: "uis-fld-labelled",
                        label { class: "uis-fld-labelled-label", "{s_height}" }
                        select {
                            class: "uis-ddn-std",
                            value: "{max_height.map(|h| h.to_string()).unwrap_or_default()}",
                            onchange: move |e| {
                                let h = e.value().parse::<u32>().ok();
                                edit_preset(settings, audio, &id_for_height, |p| {
                                    if let Mode::Video { max_height, .. } = &mut p.mode {
                                        *max_height = h;
                                    }
                                });
                            },
                            option { value: "", selected: max_height.is_none(), "{s_best}" }
                            for h in heights.iter().copied() {
                                option { key: "{h}", value: "{h}", selected: max_height == Some(h), "{h}p" }
                            }
                        }
                    }
                    div { class: "uis-fld-labelled",
                        label { class: "uis-fld-labelled-label", "{s_container}" }
                        select {
                            class: "uis-ddn-std",
                            value: "{container.id()}",
                            onchange: move |e| {
                                if let Some(c) = Container::from_id(&e.value()) {
                                    edit_preset(settings, audio, &id_for_container, |p| {
                                        if let Mode::Video { container, .. } = &mut p.mode {
                                            *container = c;
                                        }
                                    });
                                }
                            },
                            for c in Container::ALL {
                                option { key: "{c.id()}", value: "{c.id()}", selected: c == container,
                                    if c == Container::Best { "{s_format_best}" } else { "{c.label()}" }
                                }
                            }
                        }
                    }
                }
                if let Some(format) = format {
                    div { class: "uis-fld-labelled",
                        label { class: "uis-fld-labelled-label", "{s_format}" }
                        select {
                            class: "uis-ddn-std",
                            value: "{format.id()}",
                            onchange: move |e| {
                                if let Some(f) = AudioFormat::from_id(&e.value()) {
                                    edit_preset(settings, audio, &id_for_format, |p| p.mode = Mode::Audio { format: f });
                                }
                            },
                            for f in AudioFormat::ALL {
                                option { key: "{f.id()}", value: "{f.id()}", selected: f == format,
                                    if f == AudioFormat::Best { "{s_format_best}" } else { "{f.label()}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn edit_destination(mut settings: Signal<Settings>, id: &str, edit: impl FnOnce(&mut Destination)) {
    let mut s = settings.write();
    if let Some(d) = s.destinations.iter_mut().find(|d| d.id == id) {
        edit(d);
    }
}

/// Which of a destination's two patterns a box holds.
#[derive(Clone, Copy, PartialEq)]
enum Pattern {
    File,
    Set,
}

/// The id of a pattern's text box, so a click on a name can ask the box where its caret is.
fn pattern_box_id(which: Pattern, id: &str) -> String {
    match which {
        Pattern::File => format!("dst-file-{id}"),
        Pattern::Set => format!("dst-set-{id}"),
    }
}

/// A click on a name puts it in the pattern AT THE CARET (over the selection, when there is one), not at the end:
/// the box is asked where its caret is (a box keeps it while the button has the focus), the name is spliced in, and the
/// caret goes back after it, in the box.
fn add_token(settings: Signal<Settings>, which: Pattern, id: String, token: &'static str) {
    spawn(async move {
        let box_id = pattern_box_id(which, &id);
        let mut ev = document::eval(&format!(
            "const b = document.getElementById('{box_id}'); dioxus.send(b ? [b.selectionStart, b.selectionEnd] : [-1, -1]);"
        ));
        let (from, to) = match ev.recv::<[i64; 2]>().await {
            Ok([a, b]) if a >= 0 && b >= 0 => (a as usize, b as usize),
            _ => (usize::MAX, usize::MAX),
        };
        let current = settings.read().destinations.iter().find(|d| d.id == id).map(|d| match which {
            Pattern::File => d.pattern.clone(),
            Pattern::Set => d.set_pattern.clone(),
        });
        let Some(text) = current else { return };
        let (next, caret) = with_token(&text, from, to, token);
        edit_destination(settings, &id, |d| match which {
            Pattern::File => d.pattern = next,
            Pattern::Set => d.set_pattern = next,
        });
        // After the page has taken the new text: back into the box, the caret after the name just placed.
        let _ = document::eval(&format!(
            "setTimeout(() => {{ const b = document.getElementById('{box_id}'); if (b) {{ b.focus(); b.setSelectionRange({caret}, {caret}); }} }}, 30);"
        ));
    });
}

/// `text` with `{token}` put over the span from..to (positions in UTF-16 units, as a box reports them; either one past
/// the end, or `usize::MAX`, = the end), and the caret position (UTF-16 units) just after the name.
fn with_token(text: &str, from: usize, to: usize, token: &str) -> (String, usize) {
    let byte = |pos: usize| {
        let mut units = 0usize;
        for (i, ch) in text.char_indices() {
            if units >= pos {
                return i;
            }
            units += ch.len_utf16();
        }
        text.len()
    };
    let (a, b) = (byte(from.min(to)), byte(from.max(to)));
    let name = format!("{{{token}}}");
    let caret = text[..a].encode_utf16().count() + name.encode_utf16().count();
    (format!("{}{}{}", &text[..a], name, &text[b..]), caret)
}

/// `audio`: which list the window shows. A destination clicked here becomes the pick of that list.
#[component]
pub fn DestinationEditor(settings: Signal<Settings>, audio: bool) -> Element {
    let mut selected = use_signal(|| settings.read().destination_id_of(audio).to_owned());
    let mut dragging = use_signal(|| Option::<String>::None);

    let lang = settings.read().lang.clone();
    let t = |key: &str| i18n::t(&lang, key);

    let dests = settings.read().destinations.clone();
    let current = dests.iter().find(|d| d.id == selected()).unwrap_or(&dests[0]).clone();
    let cur_id = current.id.clone();
    let can_delete = dests.len() > 1;

    let s_new = t("btnMngNew");
    let s_dup = t("btnMngDuplicate");
    let s_del = t("btnMngDelete");
    let s_reorder = t("msgMngReorderHint");
    let s_name = t("lblDstName");
    let s_folder = t("lblDstFolder");
    let s_browse = t("btnDstBrowse");
    let s_pattern = t("lblDstFileName");
    let s_hint = t("msgDstNameHint");
    let s_preview = t("lblDstPreview");
    let s_downloads = t("msgDstDownloadsFolder");

    let id_for_name = cur_id.clone();
    let id_for_folder = cur_id.clone();
    let id_for_browse = cur_id.clone();
    let id_for_pattern = cur_id.clone();
    let id_for_set = cur_id.clone();
    let s_set = t("lblDstSetFolder");
    let s_set_hint = t("msgDstSetFolderHint");
    let id_for_dup = cur_id.clone();
    let id_for_del = cur_id.clone();
    let example = format!(
        "{}/{}{}",
        settings::folder_of(&current).display(),
        if current.set_folder { format!("{}/", destination::set_preview(&current.set_pattern)) } else { String::new() },
        destination::preview(&current.pattern)
    );
    let id_for_set_pattern = cur_id.clone();
    let s_set_pattern = t("lblDstSetName");
    let s_set_pattern_hint = t("msgDstSetPatternHint");

    rsx! {
        div { class: "manager",
            div { class: "manager-list uis-lvw-reorder",
                div { class: "manager-actions",
                    button {
                        class: "uis-btn ico",
                        title: "{s_new}",
                        "aria-label": "{s_new}",
                        onclick: move |_| {
                            let d = Destination::new(&s_new, "");
                            selected.set(d.id.clone());
                            settings.write().destinations.push(d);
                        },
                        Icon { width: 14, height: 14, icon: LdPlus }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_dup}",
                        "aria-label": "{s_dup}",
                        onclick: move |_| {
                            let src = settings.read().destinations.iter().find(|d| d.id == id_for_dup).cloned();
                            if let Some(mut d) = src {
                                d.id = preset::new_id();
                                d.name = format!("{} 2", d.name);
                                selected.set(d.id.clone());
                                settings.write().destinations.push(d);
                            }
                        },
                        Icon { width: 14, height: 14, icon: LdCopy }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_del}",
                        "aria-label": "{s_del}",
                        disabled: !can_delete,
                        onclick: move |_| {
                            settings.write().destinations.retain(|d| d.id != id_for_del);
                            let first = settings.read().destinations[0].id.clone();
                            selected.set(first.clone());
                            // The one picked in the panel (for either list) may be the one just deleted.
                            for kind in [false, true] {
                                let gone = !settings.read().destinations.iter().any(|d| d.id == settings.read().destination_id_of(kind));
                                if gone && settings.read().one_off(kind).is_none_or(|one| one.id != settings.read().destination_id_of(kind)) {
                                    settings.write().set_destination(kind, first.clone());
                                }
                            }
                        },
                        Icon { width: 14, height: 14, icon: LdTrash2 }
                    }
                }
                div { class: "uis-lvw-scroll",
                    for d in dests.iter() {
                        {
                            let id = d.id.clone();
                            let id_down = d.id.clone();
                            let id_over = d.id.clone();
                            let active = d.id == cur_id;
                            let moving = dragging().as_deref() == Some(d.id.as_str());
                            let shown = if d.folder.trim().is_empty() { s_downloads.clone() } else { d.folder.clone() };
                            rsx! {
                                button {
                                    key: "{d.id}",
                                    class: match (active, moving) {
                                        (_, true) => "uis-lvw-item reordering",
                                        (true, _) => "uis-lvw-item active",
                                        _ => "uis-lvw-item",
                                    },
                                    onpointerdown: move |e| {
                                        if e.trigger_button() == Some(MouseButton::Primary) {
                                            dragging.set(Some(id_down.clone()));
                                        }
                                    },
                                    onpointerenter: move |e| {
                                        let Some(from) = dragging() else { return };
                                        if !e.held_buttons().contains(MouseButton::Primary) {
                                            dragging.set(None);
                                            return;
                                        }
                                        reorder(&mut settings.write().destinations, |d| d.id.as_str(), &from, &id_over);
                                    },
                                    onpointerup: move |_| dragging.set(None),
                                    onclick: move |_| {
                                        selected.set(id.clone());
                                        settings.write().set_destination(audio, id.clone());
                                    },
                                    div { class: "uis-lvw-item-body",
                                        div { class: "uis-lvw-item-name", "{d.name}" }
                                        div { class: "uis-lvw-item-sub", "{shown}" }
                                    }
                                }
                            }
                        }
                    }
                }
                p { class: "uis-hnt", "{s_reorder}" }
            }
            div { class: "manager-form",
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_name}" }
                    input {
                        class: "uis-txt",
                        value: "{current.name}",
                        oninput: move |e| {
                            let v = e.value();
                            edit_destination(settings, &id_for_name, |d| d.name = v);
                        },
                    }
                }
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_folder}" }
                    div { class: "manager-row",
                        input {
                            class: "uis-txt",
                            value: "{current.folder}",
                            placeholder: "{s_downloads}",
                            oninput: move |e| {
                                let v = e.value();
                                edit_destination(settings, &id_for_folder, |d| d.folder = v);
                            },
                        }
                        button {
                            class: "uis-btn",
                            onclick: move |_| {
                                let id = id_for_browse.clone();
                                let window = dioxus::desktop::window();
                                spawn(async move {
                                    if let Some(dir) = rfd::AsyncFileDialog::new().set_parent(&window.window).pick_folder().await {
                                        let v = dir.path().display().to_string();
                                        edit_destination(settings, &id, |d| d.folder = v);
                                    }
                                });
                            },
                            Icon { width: 14, height: 14, icon: LdFolderOpen }
                            "{s_browse}"
                        }
                    }
                }
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_pattern}" }
                    input {
                        id: "{pattern_box_id(Pattern::File, &cur_id)}",
                        class: "uis-txt",
                        value: "{current.pattern}",
                        oninput: move |e| {
                            let v = e.value();
                            edit_destination(settings, &id_for_pattern, |d| d.pattern = v);
                        },
                    }
                    div { class: "manager-tokens",
                        for token in destination::TOKENS {
                            {
                                let id = cur_id.clone();
                                rsx! {
                                    button {
                                        key: "{token}",
                                        class: "uis-chp",
                                        onclick: move |_| add_token(settings, Pattern::File, id.clone(), token),
                                        "{token}"
                                    }
                                }
                            }
                        }
                    }
                    p { class: "uis-hnt", "{s_hint}" }
                }
                div { class: "uis-fld-labelled",
                    label { class: "uis-chk-lbl",
                        input {
                            r#type: "checkbox",
                            class: "uis-chk-box",
                            checked: current.set_folder,
                            onchange: move |e| {
                                let on = e.checked();
                                edit_destination(settings, &id_for_set, |d| d.set_folder = on);
                            },
                        }
                        "{s_set}"
                    }
                    p { class: "uis-hnt", "{s_set_hint}" }
                }
                if current.set_folder {
                    div { class: "uis-fld-labelled",
                        label { class: "uis-fld-labelled-label", "{s_set_pattern}" }
                        input {
                            id: "{pattern_box_id(Pattern::Set, &cur_id)}",
                            class: "uis-txt",
                            value: "{current.set_pattern}",
                            oninput: move |e| {
                                let v = e.value();
                                edit_destination(settings, &id_for_set_pattern, |d| d.set_pattern = v);
                            },
                        }
                        div { class: "manager-tokens",
                            for token in destination::SET_TOKENS {
                                {
                                    let id = cur_id.clone();
                                    rsx! {
                                        button {
                                            key: "{token}",
                                            class: "uis-chp",
                                            onclick: move |_| add_token(settings, Pattern::Set, id.clone(), token),
                                            "{token}"
                                        }
                                    }
                                }
                            }
                        }
                        p { class: "uis-hnt", "{s_set_pattern_hint}" }
                    }
                }
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_preview}" }
                    div { class: "manager-example", "{example}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::with_token;

    #[test]
    fn a_name_goes_in_at_the_caret_not_at_the_end() {
        assert_eq!(with_token("{title}", 0, 0, "track"), ("{track}{title}".to_owned(), 7));
        assert_eq!(with_token("{track}-{title}", 7, 7, "id"), ("{track}{id}-{title}".to_owned(), 11));
        assert_eq!(with_token("{track}-{title}", 15, 15, "id"), ("{track}-{title}{id}".to_owned(), 19));
    }

    #[test]
    fn a_selection_is_replaced_and_a_caret_past_the_end_means_the_end() {
        assert_eq!(with_token("{track}-{title}", 7, 8, "id"), ("{track}{id}{title}".to_owned(), 11));
        assert_eq!(with_token("{title}", 4, 2, "id"), ("{t{id}le}".to_owned(), 6));
        assert_eq!(with_token("{title}", usize::MAX, usize::MAX, "id"), ("{title}{id}".to_owned(), 11));
        assert_eq!(with_token("", 0, 0, "id"), ("{id}".to_owned(), 4));
    }

    #[test]
    fn positions_are_utf16_units_as_the_box_reports_them() {
        // "é" is one unit, "𝄞" (outside the BMP) is two: a caret after both is at 3.
        assert_eq!(with_token("é𝄞x", 3, 3, "id"), ("é𝄞{id}x".to_owned(), 7));
    }
}

/// The key of the Settings tab of the traffic sets.
pub const TRAFFIC: &str = "traffic";

/// The limit a set gets when "Unlimited" is unticked: a number to start from, in KB per second.
const FIRST_LIMIT_KBPS: u32 = 500;

fn edit_traffic(mut settings: Signal<Settings>, id: &str, edit: impl FnOnce(&mut Traffic)) {
    let mut s = settings.write();
    if let Some(t) = s.traffics.iter_mut().find(|t| t.id == id) {
        edit(t);
    }
}

/// What a set says under its name in the list: its limit and how many download at once.
fn traffic_summary(set: &Traffic, unlimited: &str, at_once: &str) -> String {
    let limit = match set.limit_kbps {
        Some(k) => format!("{k} KB/s"),
        None => unlimited.to_owned(),
    };
    format!("{limit} · {}", at_once.replace("{n}", &set.parallel.to_string()))
}

/// Settings > Traffic limits: the sets the panel's Traffic drop-down offers, each a limit for all the downloads together and a count of
/// downloads at once. A list kept like the destinations: new, duplicate, delete, drag to reorder; a click picks the set in the panel too.
#[component]
pub fn TrafficEditor(settings: Signal<Settings>) -> Element {
    let mut selected = use_signal(|| settings.read().traffic_id.clone());
    let mut dragging = use_signal(|| Option::<String>::None);

    let lang = settings.read().lang.clone();
    let t = |key: &str| i18n::t(&lang, key);

    let sets = settings.read().traffics.clone();
    let current = sets.iter().find(|x| x.id == selected()).unwrap_or(&sets[0]).clone();
    let cur_id = current.id.clone();
    let can_delete = sets.len() > 1;
    let unlimited = current.limit_kbps.is_none();

    let s_new = t("btnMngNew");
    let s_dup = t("btnMngDuplicate");
    let s_del = t("btnMngDelete");
    let s_reorder = t("msgMngReorderHint");
    let s_name = t("lblTrfName");
    let s_limit = t("lblTrfLimit");
    let s_unlimited = t("lblTrfUnlimited");
    let s_limit_hint = t("msgTrfLimitHint");
    let s_parallel = t("lblTrfParallel");
    let s_at_once = t("msgTrfAtOnce");

    let id_for_name = cur_id.clone();
    let id_for_unlimited = cur_id.clone();
    let id_for_limit = cur_id.clone();
    let id_for_parallel = cur_id.clone();
    let id_for_dup = cur_id.clone();
    let id_for_del = cur_id.clone();

    rsx! {
        div { class: "manager",
            div { class: "manager-list uis-lvw-reorder",
                div { class: "manager-actions",
                    button {
                        class: "uis-btn ico",
                        title: "{s_new}",
                        "aria-label": "{s_new}",
                        onclick: move |_| {
                            let set = Traffic::new(&s_new);
                            selected.set(set.id.clone());
                            settings.write().traffics.push(set);
                        },
                        Icon { width: 14, height: 14, icon: LdPlus }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_dup}",
                        "aria-label": "{s_dup}",
                        onclick: move |_| {
                            let src = settings.read().traffics.iter().find(|x| x.id == id_for_dup).cloned();
                            if let Some(mut set) = src {
                                set.id = preset::new_id();
                                set.name = format!("{} 2", set.name);
                                selected.set(set.id.clone());
                                settings.write().traffics.push(set);
                            }
                        },
                        Icon { width: 14, height: 14, icon: LdCopy }
                    }
                    button {
                        class: "uis-btn ico",
                        title: "{s_del}",
                        "aria-label": "{s_del}",
                        disabled: !can_delete,
                        onclick: move |_| {
                            settings.write().traffics.retain(|x| x.id != id_for_del);
                            let first = settings.read().traffics[0].id.clone();
                            selected.set(first.clone());
                            // The one picked in the panel may be the one just deleted.
                            if !settings.read().traffics.iter().any(|x| x.id == settings.read().traffic_id) {
                                settings.write().traffic_id = first;
                            }
                        },
                        Icon { width: 14, height: 14, icon: LdTrash2 }
                    }
                }
                div { class: "uis-lvw-scroll",
                    for set in sets.iter() {
                        {
                            let id = set.id.clone();
                            let id_down = set.id.clone();
                            let id_over = set.id.clone();
                            let active = set.id == cur_id;
                            let moving = dragging().as_deref() == Some(set.id.as_str());
                            let summary = traffic_summary(set, &s_unlimited, &s_at_once);
                            rsx! {
                                button {
                                    key: "{set.id}",
                                    class: match (active, moving) {
                                        (_, true) => "uis-lvw-item reordering",
                                        (true, _) => "uis-lvw-item active",
                                        _ => "uis-lvw-item",
                                    },
                                    onpointerdown: move |e| {
                                        if e.trigger_button() == Some(MouseButton::Primary) {
                                            dragging.set(Some(id_down.clone()));
                                        }
                                    },
                                    onpointerenter: move |e| {
                                        let Some(from) = dragging() else { return };
                                        if !e.held_buttons().contains(MouseButton::Primary) {
                                            dragging.set(None);
                                            return;
                                        }
                                        reorder(&mut settings.write().traffics, |x| x.id.as_str(), &from, &id_over);
                                    },
                                    onpointerup: move |_| dragging.set(None),
                                    onclick: move |_| {
                                        selected.set(id.clone());
                                        settings.write().traffic_id = id.clone();
                                    },
                                    div { class: "uis-lvw-item-body",
                                        div { class: "uis-lvw-item-name", "{set.name}" }
                                        div { class: "uis-lvw-item-sub", "{summary}" }
                                    }
                                }
                            }
                        }
                    }
                }
                p { class: "uis-hnt", "{s_reorder}" }
            }
            div { class: "manager-form",
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_name}" }
                    input {
                        class: "uis-txt",
                        value: "{current.name}",
                        oninput: move |e| {
                            let v = e.value();
                            edit_traffic(settings, &id_for_name, |x| x.name = v);
                        },
                    }
                }
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_limit}" }
                    div { class: "manager-row",
                        label { class: "uis-chk-lbl",
                            input {
                                r#type: "checkbox",
                                class: "uis-chk-box",
                                checked: unlimited,
                                onchange: move |e| {
                                    let on = e.checked();
                                    edit_traffic(settings, &id_for_unlimited, |x| x.limit_kbps = if on { None } else { Some(FIRST_LIMIT_KBPS) });
                                },
                            }
                            "{s_unlimited}"
                        }
                        UisNum {
                            value: current.limit_kbps.map(f64::from),
                            min: Some(1.0),
                            max: Some(1_000_000.0),
                            step: 50.0,
                            decimals: Some(0),
                            width: "96px".to_owned(),
                            disabled: unlimited,
                            on_change: move |v: Option<f64>| {
                                if let Some(v) = v {
                                    edit_traffic(settings, &id_for_limit, |x| x.limit_kbps = Some(v.round().max(1.0) as u32));
                                }
                            },
                        }
                        span { "KB/s" }
                    }
                    p { class: "uis-hnt", "{s_limit_hint}" }
                }
                div { class: "uis-fld-labelled",
                    label { class: "uis-fld-labelled-label", "{s_parallel}" }
                    select {
                        class: "uis-ddn-std",
                        value: "{current.parallel}",
                        onchange: move |e| {
                            if let Ok(n) = e.value().parse::<u32>() {
                                edit_traffic(settings, &id_for_parallel, |x| x.parallel = n.clamp(1, traffic::MAX_PARALLEL));
                            }
                        },
                        for n in 1..=traffic::MAX_PARALLEL {
                            option { key: "{n}", value: "{n}", selected: n == current.parallel, "{n}" }
                        }
                    }
                }
            }
        }
    }
}
