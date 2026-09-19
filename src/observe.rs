//! Test-only observation hooks used to verify which internal path a detection
//! takes. This module is compiled only under `cfg(test)` and is never part of
//! the public or release API.
use std::cell::Cell;

/// Which branch of `detect_by_query` was taken for the main script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetectPath {
    /// Script maps to exactly one language; no scoring runs at all.
    OneLang,
    /// Script maps to many languages; alphabet/trigram/combined scoring runs.
    Multi,
    /// Mandarin script heuristic (Cmn vs Jpn by kana ratio).
    Mandarin,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Observation {
    pub path: Option<DetectPath>,
    /// Whether the trigram stage actually ran for this detection.
    pub trigram_computed: bool,
    /// Number of candidate languages that survived the filter list in the
    /// trigram stage.
    pub trigram_candidates: Option<usize>,
    /// Number of unique trigrams extracted from the text.
    pub trigrams_count: Option<usize>,
}

thread_local! {
    static CURRENT: Cell<Observation> = const { Cell::new(Observation {
        path: None,
        trigram_computed: false,
        trigram_candidates: None,
        trigrams_count: None,
    }) };
}

pub(crate) fn reset() {
    CURRENT.with(|c| c.set(Observation::default()));
}

pub(crate) fn snapshot() -> Observation {
    CURRENT.with(|c| c.get())
}

pub(crate) fn note_path(path: DetectPath) {
    CURRENT.with(|c| {
        let mut obs = c.get();
        obs.path = Some(path);
        c.set(obs);
    });
}

pub(crate) fn note_trigram(candidates: usize, trigrams_count: usize) {
    CURRENT.with(|c| {
        let mut obs = c.get();
        obs.trigram_computed = true;
        obs.trigram_candidates = Some(candidates);
        obs.trigrams_count = Some(trigrams_count);
        c.set(obs);
    });
}
