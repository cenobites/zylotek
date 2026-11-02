//! Whatever a language model says: the reply parser never panics, what it accepts obeys the
//! documented bounds, and ranking against any figures (NaN and infinity included) keeps its
//! scores in 0..=1, in order, and within the limit.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use zylotek_assistant::{ranking::Ranker, requirements::Requirements};
use zylotek_contracts::woods::{WoodDetailsSchema, WoodResponse};

#[derive(Debug, Arbitrary)]
struct Input {
    reply: String,
    /// (Janka hardness, volumetric shrinkage) of each wood in the catalogue.
    figures: Vec<(Option<f64>, Option<f64>)>,
    limit: u8,
}

fuzz_target!(|input: Input| {
    let Ok(requirements) = Requirements::parse(&input.reply) else {
        return;
    };

    assert!(!requirements.criteria.is_empty());
    assert!(requirements.summary.chars().count() <= 200);
    assert!(requirements.avoid.len() <= 10);
    let mut seen = Vec::new();
    for criterion in &requirements.criteria {
        assert!((1..=3).contains(&criterion.weight));
        assert!(!seen.contains(&criterion.property), "duplicate property");
        seen.push(criterion.property);
    }

    let woods = input
        .figures
        .iter()
        .enumerate()
        .map(|(n, (janka, shrinkage))| {
            let mut details = WoodDetailsSchema::default();
            details
                .mechanical_properties
                .dry
                .janka_hardness
                .parallel_to_grain = *janka;
            details.physical_properties.shrinkage.volumetric = *shrinkage;

            WoodResponse {
                id: n.to_string(),
                name: format!("wood {n}"),
                scientific_name: format!("genus species{n}"),
                genus: "genus".into(),
                details,
            }
        })
        .collect();
    let ranked = Ranker::new(woods).rank(&requirements, usize::from(input.limit));

    assert!(ranked.len() <= usize::from(input.limit));
    for recommendation in &ranked {
        assert!(
            (0.0..=1.0).contains(&recommendation.score),
            "{}",
            recommendation.score
        );
        assert!(!recommendation.reasons.is_empty());
        for reason in &recommendation.reasons {
            assert!((0.0..=1.0).contains(&reason.fit));
            assert!(reason.value.is_finite());
        }
    }
    assert!(ranked.windows(2).all(|pair| pair[0].score >= pair[1].score));
});
