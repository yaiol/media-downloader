//! A traffic set: how fast the downloads may go all together, and how many run at once. The user keeps a list of them in Settings and
//! picks one in the panel (as with the destinations). The limit is a TOTAL: the runner shares it between the downloads running at that
//! moment (`runner::share`), so two at once under 500 KB/s get 250 KB/s each.

use serde::{Deserialize, Serialize};

/// The most downloads that run at once. A download is bound by the network, not the processor, so this is a courtesy to the sites.
pub const MAX_PARALLEL: u32 = 6;

/// A KB is 1024 bytes, as yt-dlp's `K` is.
pub const KB: u64 = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Traffic {
    pub id: String,
    pub name: String,
    /// All the downloads together, in KB per second; `None` = unlimited.
    pub limit_kbps: Option<u32>,
    /// How many download at once (1 to `MAX_PARALLEL`).
    pub parallel: u32,
}

impl Default for Traffic {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), limit_kbps: None, parallel: 2 }
    }
}

impl Traffic {
    pub fn new(name: &str) -> Self {
        Self { id: crate::preset::new_id(), name: name.to_owned(), ..Self::default() }
    }

    /// The limit in bytes per second, if there is one.
    pub fn limit_bytes(&self) -> Option<u64> {
        self.limit_kbps.filter(|k| *k > 0).map(|k| u64::from(k) * KB)
    }
}

/// What a fresh install holds: no limit, and a slow one that stays out of the way.
pub fn seeded() -> Vec<Traffic> {
    vec![
        Traffic { id: "traffic-unlimited".to_owned(), name: "Unlimited".to_owned(), limit_kbps: None, parallel: 2 },
        Traffic { id: "traffic-background".to_owned(), name: "Background".to_owned(), limit_kbps: Some(500), parallel: 1 },
    ]
}

/// The list as it should be kept: never empty, no repeated id, no blank name, a count from 1 to `MAX_PARALLEL`, and a limit of 0 means none.
pub fn repaired(list: Vec<Traffic>) -> Vec<Traffic> {
    let mut out: Vec<Traffic> = Vec::new();
    for mut t in list {
        if t.id.trim().is_empty() || out.iter().any(|x| x.id == t.id) {
            t.id = crate::preset::new_id();
        }
        if t.name.trim().is_empty() {
            t.name = t.id.clone();
        }
        t.parallel = t.parallel.clamp(1, MAX_PARALLEL);
        if t.limit_kbps == Some(0) {
            t.limit_kbps = None;
        }
        out.push(t);
    }
    if out.is_empty() {
        out = seeded();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_has_an_unlimited_set_first() {
        let s = seeded();
        assert_eq!((s[0].limit_kbps, s[0].parallel), (None, 2));
        assert!(s.iter().all(|t| (1..=MAX_PARALLEL).contains(&t.parallel)));
    }

    #[test]
    fn a_list_is_repaired_never_empty_unique_named_and_in_range() {
        assert_eq!(repaired(Vec::new()), seeded());
        let r = repaired(vec![
            Traffic { id: "a".into(), name: " ".into(), limit_kbps: Some(0), parallel: 0 },
            Traffic { id: "a".into(), name: "b".into(), limit_kbps: Some(300), parallel: 99 },
        ]);
        assert_eq!(r[0].id, "a");
        assert_eq!((r[0].name.as_str(), r[0].limit_kbps, r[0].parallel), ("a", None, 1));
        assert_ne!(r[1].id, "a", "a repeated id is replaced");
        assert_eq!((r[1].limit_kbps, r[1].parallel), (Some(300), MAX_PARALLEL));
    }

    #[test]
    fn the_limit_is_in_bytes_and_none_means_unlimited() {
        assert_eq!(Traffic { limit_kbps: Some(500), ..Traffic::default() }.limit_bytes(), Some(512_000));
        assert_eq!(Traffic::default().limit_bytes(), None);
    }
}
