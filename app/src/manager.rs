//! The preset editor, a page of the Settings view: the list on the left (drag a row to reorder, new, duplicate, delete), the options of
//! the selected preset on the right. Every change is stored at once, there is no Save button. The saved order is the order of the
//! panel's drop-down too.
//!
//! ⚠ CLAUDE: NO HARDCODED UI STRINGS. File types (MP4, MKV, WebM, MP3, M4A, Opus) and heights (1080p) are names and stay literal;
//! everything else goes through the t() lookup with a literal key at the call site.

use crate::{
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
    let s_pattern = t("lblDstPattern");
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
    let s_set_pattern = t("lblDstSetPattern");
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
                                let base = current.pattern.clone();
                                rsx! {
                                    button {
                                        key: "{token}",
                                        class: "uis-chp",
                                        onclick: move |_| {
                                            let next = format!("{base}{{{token}}}");
                                            edit_destination(settings, &id, |d| d.pattern = next);
                                        },
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
                                    let base = current.set_pattern.clone();
                                    rsx! {
                                        button {
                                            key: "{token}",
                                            class: "uis-chp",
                                            onclick: move |_| {
                                                let next = format!("{base}{{{token}}}");
                                                edit_destination(settings, &id, |d| d.set_pattern = next);
                                            },
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
