//! Test-only observation hooks for the `understanding` tests.
//!
//! This module is compiled only under `cfg(test)`. It never becomes part of
//! the release API. It does not reimplement any detection logic: it records
//! which branch the real `detect_by_query` dispatch took and how many
//! candidates the real trigram scorer saw after filtering.

use std::cell::Cell;

use crate::Lang;
use crate::core::{Options, detect_with_options};
use crate::scripts::grouping::ScriptLangGroup;

/// Which branch of `detect_by_query` was taken for a given input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedPath {
    /// Script maps to exactly one language (e.g. Greek -> Ell).
    One,
    /// Multi-language script: alphabet/trigram/combined scoring ran.
    Multi,
    /// Mandarin/Japanese heuristic based on kana counts.
    Mandarin,
}

/// What actually happened inside a single `detect_with_options` call.
#[derive(Debug)]
pub(crate) struct Observation {
    /// Dispatch branch taken in `detect_by_query`; `None` if the text has no
    /// dominant script and detection bailed out before dispatching.
    pub path: Option<ObservedPath>,
    /// Final language returned by the real detection pipeline.
    pub lang: Option<Lang>,
    /// Whether the trigram scorer ran at all.
    pub trigram_computed: bool,
    /// Number of language candidates the trigram scorer kept after applying
    /// the filter list (0 when the trigram scorer did not run).
    pub trigram_candidates: usize,
}

thread_local! {
    static PATH: Cell<Option<ObservedPath>> = const { Cell::new(None) };
    static TRIGRAM_RUNS: Cell<usize> = const { Cell::new(0) };
    static TRIGRAM_CANDIDATES: Cell<usize> = const { Cell::new(0) };
}

/// Called from `detect_by_query` (src/core/detect.rs) under `cfg(test)`.
pub(crate) fn note_lang_group(group: &ScriptLangGroup) {
    let path = match group {
        ScriptLangGroup::One(_) => ObservedPath::One,
        ScriptLangGroup::Multi(_) => ObservedPath::Multi,
        ScriptLangGroup::Mandarin => ObservedPath::Mandarin,
    };
    PATH.with(|p| p.set(Some(path)));
}

/// Called from `calculate_scores_in_profiles` (src/trigrams/detection.rs)
/// under `cfg(test)` with the number of candidates left after filtering.
pub(crate) fn note_trigram_run(candidates: usize) {
    TRIGRAM_RUNS.with(|c| c.set(c.get() + 1));
    TRIGRAM_CANDIDATES.with(|c| c.set(candidates));
}

fn reset() {
    PATH.with(|p| p.set(None));
    TRIGRAM_RUNS.with(|c| c.set(0));
    TRIGRAM_CANDIDATES.with(|c| c.set(0));
}

/// Run the real public detection entry point and report what was observed.
pub(crate) fn observe(text: &str, options: &Options) -> Observation {
    reset();
    let info = detect_with_options(text, options);
    Observation {
        path: PATH.with(|p| p.get()),
        lang: info.map(|i| i.lang()),
        trigram_computed: TRIGRAM_RUNS.with(|c| c.get()) > 0,
        trigram_candidates: TRIGRAM_CANDIDATES.with(|c| c.get()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::FilterList;

    // Latin-majority text with an embedded Cyrillic word: the main script is
    // Latin, so detection goes through the multi-language (Combined) path.
    const MIXED_TEXT: &str =
        "This English sentence quietly contains the Russian word любовь inside it.";

    #[test]
    fn understanding_mixed_text_unfiltered_uses_all_latin_candidates() {
        let obs = observe(MIXED_TEXT, &Options::default());
        assert_eq!(obs.path, Some(ObservedPath::Multi));
        assert!(obs.trigram_computed);
        // All 37 Latin trigram profiles survive the default FilterList::All.
        assert_eq!(obs.trigram_candidates, 37);
        assert_eq!(obs.lang, Some(Lang::Eng));
    }

    #[test]
    fn understanding_mixed_text_allowlist_narrows_candidates() {
        let options = Options::new().set_filter_list(FilterList::allow(vec![Lang::Eng, Lang::Deu]));
        let obs = observe(MIXED_TEXT, &options);
        // Same dispatch path and the trigram scorer still runs...
        assert_eq!(obs.path, Some(ObservedPath::Multi));
        assert!(obs.trigram_computed);
        // ...but only the two allowlisted profiles are scored.
        assert_eq!(obs.trigram_candidates, 2);
        assert_eq!(obs.lang, Some(Lang::Eng));
    }

    #[test]
    fn understanding_mixed_text_denylist_excludes_winner() {
        let options = Options::new().set_filter_list(FilterList::deny(vec![Lang::Eng]));
        let obs = observe(MIXED_TEXT, &options);
        assert_eq!(obs.path, Some(ObservedPath::Multi));
        assert!(obs.trigram_computed);
        // 37 Latin profiles minus the denied one.
        assert_eq!(obs.trigram_candidates, 36);
        // With Eng excluded, the runner-up (Fra) wins instead.
        assert_eq!(obs.lang, Some(Lang::Fra));
    }

    #[test]
    fn understanding_single_language_script_skips_trigram() {
        // Greek maps to exactly one language (Ell); confidence is hardcoded.
        let obs = observe("Η ελληνική γλώσσα", &Options::default());
        assert_eq!(obs.path, Some(ObservedPath::One));
        assert!(!obs.trigram_computed);
        assert_eq!(obs.trigram_candidates, 0);
        assert_eq!(obs.lang, Some(Lang::Ell));
    }

    #[test]
    fn understanding_mandarin_heuristic_skips_trigram() {
        // Mixed Mandarin + Hiragana: resolved by kana counts, not trigrams.
        let obs = observe("東京は水と木", &Options::default());
        assert_eq!(obs.path, Some(ObservedPath::Mandarin));
        assert!(!obs.trigram_computed);
        assert_eq!(obs.trigram_candidates, 0);
        assert_eq!(obs.lang, Some(Lang::Jpn));
    }
}
