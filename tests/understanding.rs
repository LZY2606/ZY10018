//! Verification-stage runner for the `understanding` acceptance command.
//!
//! This target uses `harness = false` so its stage names are printed directly
//! in the output of `cargo test --quiet understanding`. The fine-grained
//! assertions (dispatch path, candidate counts, trigram usage) live in the
//! `understanding_*` unit tests in src/core/probe.rs; this runner re-checks
//! the same mixed-text scenarios through the public API only.

use whatlang::{Detector, Lang, detect};

const MIXED_TEXT: &str =
    "This English sentence quietly contains the Russian word любовь inside it.";

fn main() {
    println!("understanding stage: unfiltered mixed text -> Eng");
    assert_eq!(detect(MIXED_TEXT).map(|info| info.lang()), Some(Lang::Eng));

    println!("understanding stage: allowlist [Eng, Deu] narrows candidates -> Eng");
    let detector = Detector::with_allowlist(vec![Lang::Eng, Lang::Deu]);
    assert_eq!(detector.detect_lang(MIXED_TEXT), Some(Lang::Eng));

    println!("understanding stage: denylist [Eng] excludes the winner -> Fra");
    let detector = Detector::with_denylist(vec![Lang::Eng]);
    assert_eq!(detector.detect_lang(MIXED_TEXT), Some(Lang::Fra));

    println!("understanding stage: see src/core/probe.rs unit tests for path/candidate assertions");
    println!("understanding: all stages passed");
}
