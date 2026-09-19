//! Tests that pin down the behavior documented in ANALYSIS.md.
//! All test names contain "understanding" so they can be run with:
//! `cargo test --quiet understanding`
#[cfg(test)]
mod understanding_tests {
    use crate::core::{FilterList, Options, detect_with_options};
    use crate::observe::{self, DetectPath, Observation};
    use crate::{Info, Lang};

    fn run(text: &str, options: &Options) -> (Option<Info>, Observation) {
        observe::reset();
        let info = detect_with_options(text, options);
        (info, observe::snapshot())
    }

    /// `cargo test --quiet` renders tests as dots; write the case name
    /// directly to stderr (bypassing libtest's output capture) so the
    /// acceptance run shows which cases executed.
    fn show_name(name: &str) {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "case {name} ... running");
    }

    /// Same mixed text under three filter configurations: the script path and
    /// the trigram stage are identical; only the candidate set (and thus the
    /// winner and confidence) changes.
    #[test]
    fn understanding_mixed_text_paths() {
        show_name("understanding_mixed_text_paths");
        let text = "I love you. Je t'aime. Ich liebe dich. 愛してる。";
        let latin_profile_count = crate::trigrams::LATIN_LANGS.len();

        // 1. No filter: every Latin-profile language is a candidate.
        let (info, obs) = run(text, &Options::default());
        let info = info.unwrap();
        assert_eq!(obs.path, Some(DetectPath::Multi));
        assert!(obs.trigram_computed);
        assert_eq!(obs.trigram_candidates, Some(latin_profile_count));
        assert_eq!(info.lang(), Lang::Deu);
        assert!(info.confidence() < 1.0);
        assert!(!info.is_reliable());

        // 2. Allowlist narrows candidates to exactly one; confidence is 1.0
        //    because there is no runner-up, not because the text matches well.
        let opts = Options::new().set_filter_list(FilterList::allow(vec![Lang::Eng]));
        let (info, obs) = run(text, &opts);
        let info = info.unwrap();
        assert_eq!(obs.path, Some(DetectPath::Multi));
        assert!(obs.trigram_computed);
        assert_eq!(obs.trigram_candidates, Some(1));
        assert_eq!(info.lang(), Lang::Eng);
        assert_eq!(info.confidence(), 1.0);

        // 3. Denylist removes the top three winners of the unfiltered run;
        //    the next candidate wins with low confidence.
        let opts = Options::new()
            .set_filter_list(FilterList::deny(vec![Lang::Eng, Lang::Fra, Lang::Deu]));
        let (info, obs) = run(text, &opts);
        let info = info.unwrap();
        assert_eq!(obs.path, Some(DetectPath::Multi));
        assert!(obs.trigram_computed);
        assert_eq!(obs.trigram_candidates, Some(latin_profile_count - 3));
        assert_eq!(info.lang(), Lang::Ces);
        assert!(info.confidence() < 1.0);
    }

    /// Inputs that never reach the trigram stage: single-language scripts,
    /// the Mandarin heuristic, and stop-char-only text.
    #[test]
    fn understanding_skips_trigram_paths() {
        show_name("understanding_skips_trigram_paths");
        // Hangul maps to exactly one language: no scoring at all.
        let (info, obs) = run("한국어는 한국에서 사용되는 언어입니다", &Options::default());
        let info = info.unwrap();
        assert_eq!(obs.path, Some(DetectPath::OneLang));
        assert!(!obs.trigram_computed);
        assert_eq!(obs.trigram_candidates, None);
        assert_eq!(info.lang(), Lang::Kor);
        assert_eq!(info.confidence(), 1.0);

        // Mandarin script uses the kana-ratio heuristic, not trigrams.
        let (info, obs) = run("水", &Options::default());
        let info = info.unwrap();
        assert_eq!(obs.path, Some(DetectPath::Mandarin));
        assert!(!obs.trigram_computed);
        assert_eq!(obs.trigram_candidates, None);
        assert_eq!(info.lang(), Lang::Cmn);
        assert_eq!(info.confidence(), 1.0);

        // Stop chars only: no main script, detection returns None.
        let (info, obs) = run("12345 !!!", &Options::default());
        assert_eq!(info, None);
        assert_eq!(obs.path, None);
        assert!(!obs.trigram_computed);
    }

    /// Filter-list quirks on the non-scoring paths, documented in ANALYSIS.md.
    #[test]
    fn understanding_filter_quirks() {
        show_name("understanding_filter_quirks");
        // Mandarin path: denying Cmn forces Jpn even when Jpn is also denied,
        // because the else branch never consults the filter list.
        let opts = Options::new().set_filter_list(FilterList::deny(vec![Lang::Cmn, Lang::Jpn]));
        let (info, obs) = run("水", &opts);
        let info = info.unwrap();
        assert_eq!(obs.path, Some(DetectPath::Mandarin));
        assert_eq!(info.lang(), Lang::Jpn);
        assert_eq!(info.confidence(), 1.0);

        // OneLang path: the filter list is ignored entirely.
        let opts = Options::new().set_filter_list(FilterList::deny(vec![Lang::Kor]));
        let (info, obs) = run("한국어는 한국에서 사용되는 언어입니다", &opts);
        let info = info.unwrap();
        assert_eq!(obs.path, Some(DetectPath::OneLang));
        assert_eq!(info.lang(), Lang::Kor);
    }

    /// Confidence depends on text length: short texts cannot reach 1.0 even
    /// when the top candidate is far ahead, and `is_reliable` gates at 0.9.
    #[test]
    fn understanding_confidence_thresholds() {
        show_name("understanding_confidence_thresholds");
        let (info, obs) = run("hi", &Options::default());
        let info = info.unwrap();
        assert!(obs.trigram_computed);
        assert!(obs.trigrams_count.unwrap() < 10);
        assert!(info.confidence() < 0.1);
        assert!(!info.is_reliable());

        let text = "This is a fairly long English sentence with many words in it.";
        let (info, obs) = run(text, &Options::default());
        let info = info.unwrap();
        assert!(obs.trigrams_count.unwrap() >= 10);
        assert_eq!(info.lang(), Lang::Eng);
        assert_eq!(info.confidence(), 1.0);
        assert!(info.is_reliable());
    }

    /// Repeated detection of the same text is deterministic: equal scores
    /// always resolve to the same language because the profile order is fixed
    /// and the sort is a deterministic algorithm.
    #[test]
    fn understanding_deterministic_results() {
        show_name("understanding_deterministic_results");
        let text = "I love you. Je t'aime. Ich liebe dich. 愛してる。";
        let first = detect_with_options(text, &Options::default());
        for _ in 0..10 {
            assert_eq!(detect_with_options(text, &Options::default()), first);
        }
    }
}
