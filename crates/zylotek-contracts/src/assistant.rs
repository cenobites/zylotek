//! The JSON shapes of the wood assistant: a question in plain language goes in, ranked
//! woods and a short explanation come out. Used by `zylotek-assistant` and its clients.

use serde::{Deserialize, Serialize};

/// What a wood is judged on. Each maps to numbers in the catalogue, so a ranking is
/// arithmetic over the data and the language model never decides which wood is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    /// Resists dents and wear (Janka hardness).
    Hardness,
    /// Resists breaking under bending (modulus of rupture).
    Strength,
    /// Resists sagging (modulus of elasticity).
    Stiffness,
    /// Stays flat as humidity changes (low volumetric shrinkage).
    Stability,
    /// Low density, easy to lift and carry.
    Lightness,
    /// Easy to plane, sand, clamp and drill.
    Workability,
}

impl Property {
    pub const ALL: [Property; 6] = [
        Property::Hardness,
        Property::Strength,
        Property::Stiffness,
        Property::Stability,
        Property::Lightness,
        Property::Workability,
    ];

    /// The name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Property::Hardness => "hardness",
            Property::Strength => "strength",
            Property::Stiffness => "stiffness",
            Property::Stability => "stability",
            Property::Lightness => "lightness",
            Property::Workability => "workability",
        }
    }
}

/// Body of `POST /v1/ask`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskRequest {
    pub question: String,
}

/// A property the question cares about, and how much.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Criterion {
    pub property: Property,
    /// 1 nice to have, 2 important, 3 essential.
    pub weight: u8,
}

/// One property of one recommended wood.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reason {
    pub property: Property,
    /// The raw figure: kgf for hardness, kgf/cm² for strength and stiffness, % for
    /// shrinkage, g/cm³ for density, 0 to 1 for workability.
    pub value: f64,
    /// 0 worst to 1 best among all woods with data, direction already applied.
    pub fit: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub wood_id: String,
    pub name: String,
    pub scientific_name: String,
    /// 0 to 1: the weighted fit over every criterion; a missing figure counts as zero.
    pub score: f64,
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskResponse {
    /// The question restated as a need, as the assistant understood it.
    pub summary: String,
    pub criteria: Vec<Criterion>,
    pub recommendations: Vec<Recommendation>,
    /// A short answer in the question's language; absent when the model could not write one.
    pub explanation: Option<String>,
    /// How many woods were considered.
    pub catalogue_size: usize,
}

/// The server-sent events of `POST /v1/ask/stream`, in the order they come: one `ANSWER`
/// (an `AskResponse` whose explanation is still `null`), any number of `TOKEN`s (each a JSON
/// string, the next piece of the explanation), and a closing `DONE`. A `FAILED` (a JSON
/// string saying why) just before `DONE` means the model stopped midway and the explanation
/// is incomplete.
pub mod events {
    pub const ANSWER: &str = "answer";
    pub const TOKEN: &str = "token";
    pub const FAILED: &str = "failed";
    pub const DONE: &str = "done";
}

/// Body of `GET /health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantHealth {
    pub status: String,
    /// Woods in the catalogue snapshot; 0 until the first sync.
    pub woods: usize,
    pub synced_seconds_ago: Option<u64>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn properties_are_snake_case_words_and_all_lists_each_once() {
        for property in Property::ALL {
            let json = serde_json::to_value(property).unwrap();

            assert_eq!(json, json!(property.as_str()));
            assert_eq!(serde_json::from_value::<Property>(json).unwrap(), property);
        }
        let mut names: Vec<_> = Property::ALL.iter().map(|p| p.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 6);
        assert!(serde_json::from_value::<Property>(json!("beauty")).is_err());
    }

    #[test]
    fn the_wire_types_round_trip() {
        let response = AskResponse {
            summary: "a dining table".into(),
            criteria: vec![Criterion {
                property: Property::Stability,
                weight: 3,
            }],
            recommendations: vec![Recommendation {
                wood_id: "1".into(),
                name: "Cedro".into(),
                scientific_name: "Cedrela fissilis".into(),
                score: 0.8,
                reasons: vec![Reason {
                    property: Property::Stability,
                    value: 9.5,
                    fit: 0.9,
                }],
            }],
            explanation: None,
            catalogue_size: 225,
        };

        let json = serde_json::to_string(&response).unwrap();

        assert_eq!(
            serde_json::from_str::<AskResponse>(&json).unwrap(),
            response
        );
        let request = AskRequest {
            question: "x".into(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(serde_json::from_str::<AskRequest>(&json).unwrap(), request);
        let health = AssistantHealth {
            status: "ok".into(),
            woods: 3,
            synced_seconds_ago: Some(4),
        };
        let json = serde_json::to_string(&health).unwrap();
        assert_eq!(
            serde_json::from_str::<AssistantHealth>(&json).unwrap(),
            health
        );
    }
}
