//! What the language model reads out of a question: which wood properties matter, and how
//! much. The model only translates; this module checks and tidies what it says so a
//! sloppy reply cannot reach the ranking.

use serde::Deserialize;
use serde_json::{Value, json};
use zylotek_contracts::assistant::{Criterion, Property};

use crate::error::AskError;

const MAX_WEIGHT: i64 = 3;
const MAX_AVOID: usize = 10;
const MAX_AVOID_CHARS: usize = 60;
const MAX_SUMMARY_CHARS: usize = 200;
const MAX_LANGUAGE_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirements {
    /// The need in one sentence, in the language of the question.
    pub summary: String,
    /// The language the question is written in, named in English ("Brazilian Portuguese");
    /// empty when the model did not say. The explanation is written in it.
    pub language: String,
    /// At most one entry per property, strongest weight kept, in the model's order.
    pub criteria: Vec<Criterion>,
    /// Lowercase name fragments of woods to leave out.
    pub avoid: Vec<String>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    language: String,
    priorities: Vec<RawCriterion>,
    #[serde(default)]
    avoid: Vec<String>,
}

#[derive(Deserialize)]
struct RawCriterion {
    property: Property,
    weight: i64,
}

impl Requirements {
    /// Reads the model's JSON. Tolerates prose or code fences around the object, clamps
    /// weights to 1–3, and refuses a reading with nothing to rank by.
    pub fn parse(text: &str) -> Result<Self, AskError> {
        let json = object_in(text)
            .ok_or_else(|| AskError::Understanding("no JSON object in the reply".into()))?;
        let raw: Raw = serde_json::from_str(json)
            .map_err(|error| AskError::Understanding(error.to_string()))?;

        let mut criteria: Vec<Criterion> = Vec::new();
        for entry in raw.priorities.iter().filter(|entry| entry.weight > 0) {
            let weight = entry.weight.min(MAX_WEIGHT) as u8;
            match criteria.iter_mut().find(|c| c.property == entry.property) {
                Some(existing) => existing.weight = existing.weight.max(weight),
                None => criteria.push(Criterion {
                    property: entry.property,
                    weight,
                }),
            }
        }
        if criteria.is_empty() {
            return Err(AskError::Understanding(
                "the question names nothing to rank woods by".into(),
            ));
        }

        let avoid = raw
            .avoid
            .iter()
            .map(|name| name.trim().to_lowercase())
            .filter(|name| !name.is_empty() && name.chars().count() <= MAX_AVOID_CHARS)
            .take(MAX_AVOID)
            .collect();

        Ok(Self {
            summary: raw.summary.trim().chars().take(MAX_SUMMARY_CHARS).collect(),
            language: raw
                .language
                .trim()
                .chars()
                .take(MAX_LANGUAGE_CHARS)
                .collect(),
            criteria,
            avoid,
        })
    }
}

/// The span from the first `{` to the last `}`.
fn object_in(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;

    (start < end).then(|| &text[start..=end])
}

/// The JSON schema the model's reply is forced to follow.
pub fn schema() -> Value {
    let properties: Vec<_> = Property::ALL.iter().map(|p| p.as_str()).collect();

    json!({
        "type": "object",
        "properties": {
            "language": {"type": "string"},
            "summary": {"type": "string"},
            "priorities": {
                "type": "array",
                "minItems": 1,
                "maxItems": Property::ALL.len(),
                "items": {
                    "type": "object",
                    "properties": {
                        "property": {"type": "string", "enum": properties},
                        "weight": {"type": "integer", "minimum": 1, "maximum": MAX_WEIGHT},
                    },
                    "required": ["property", "weight"],
                    "additionalProperties": false,
                },
            },
            "avoid": {"type": "array", "items": {"type": "string"}},
        },
        "required": ["language", "summary", "priorities", "avoid"],
        "additionalProperties": false,
    })
}

/// Tells the model its one job: turn the question into weighted properties.
pub const INSTRUCTIONS: &str = "\
You are the Project Wood Recommender, an expert woodworker who helps people choose wood for a project \
(decks, tables, bookshelves, floors, boats...). Read the project description and say which wood properties matter for it. \
Reply with JSON only.

Properties:
- hardness: resists dents and wear (floors, tabletops, tool handles)
- strength: resists breaking under load (beams, legs, frames)
- stiffness: resists sagging (shelves, long spans, table rails)
- stability: stays flat when the air gets humid or dry (panels, tabletops, joinery, instruments)
- lightness: low density, easy to carry (boxes, boats, portable furniture)
- workability: easy to plane, sand, drill and cut joints in (hand tools, joinery, carving)

Give each property that matters a weight: 3 essential, 2 important, 1 nice to have. \
Leave out properties that do not matter. \
\"language\" names, in English, the language the question is written in (\"English\", \"Brazilian Portuguese\", \"Spanish\"...). \
\"summary\" restates the need in one short sentence written in that same language. \
\"avoid\" lists names of woods the person does not want, lowercase; usually empty.

Example question: a dining table for 12 people with traditional Japanese joinery.
Example reply: {\"language\": \"English\", \"summary\": \"A large dining table joined without nails\", \
\"priorities\": [{\"property\": \"stability\", \"weight\": 3}, {\"property\": \"workability\", \"weight\": 3}, \
{\"property\": \"stiffness\", \"weight\": 2}, {\"property\": \"hardness\", \"weight\": 2}], \"avoid\": []}

Example question: quero um deck de 4x5 metros para uma área chuvosa.
Example reply: {\"language\": \"Brazilian Portuguese\", \"summary\": \"Um deck externo para clima chuvoso\", \
\"priorities\": [{\"property\": \"stability\", \"weight\": 3}, {\"property\": \"hardness\", \"weight\": 3}, \
{\"property\": \"strength\", \"weight\": 2}], \"avoid\": []}";

#[cfg(test)]
mod tests {
    use super::*;

    fn criteria(requirements: &Requirements) -> Vec<(Property, u8)> {
        requirements
            .criteria
            .iter()
            .map(|c| (c.property, c.weight))
            .collect()
    }

    #[test]
    fn it_reads_a_clean_reply() {
        let reply = r#"{"language": " Brazilian Portuguese ", "summary": " A table ", "priorities": [
            {"property": "stability", "weight": 3}, {"property": "hardness", "weight": 1}],
            "avoid": [" Ipê ", ""]}"#;

        let requirements = Requirements::parse(reply).unwrap();

        assert_eq!(requirements.summary, "A table");
        assert_eq!(requirements.language, "Brazilian Portuguese");
        assert_eq!(
            criteria(&requirements),
            [(Property::Stability, 3), (Property::Hardness, 1)]
        );
        assert_eq!(requirements.avoid, ["ipê"]);
    }

    #[test]
    fn it_digs_the_object_out_of_prose_and_fences() {
        let reply = "Sure! ```json\n{\"priorities\": [{\"property\": \"lightness\", \"weight\": 2}]}\n``` Done.";

        let requirements = Requirements::parse(reply).unwrap();

        assert_eq!(criteria(&requirements), [(Property::Lightness, 2)]);
        assert_eq!(requirements.summary, "", "the summary is optional");
        assert_eq!(requirements.language, "", "so is the language");
        assert!(requirements.avoid.is_empty());
    }

    #[test]
    fn weights_are_clamped_merged_and_zeros_dropped() {
        let reply = r#"{"priorities": [
            {"property": "strength", "weight": 9},
            {"property": "stiffness", "weight": 0},
            {"property": "strength", "weight": 1},
            {"property": "hardness", "weight": -4},
            {"property": "stability", "weight": 2},
            {"property": "stability", "weight": 3}]}"#;

        let requirements = Requirements::parse(reply).unwrap();

        assert_eq!(
            criteria(&requirements),
            [(Property::Strength, 3), (Property::Stability, 3)]
        );
    }

    #[test]
    fn the_avoid_list_is_lowercased_bounded_and_ignores_essays() {
        let long = "x".repeat(MAX_AVOID_CHARS + 1);
        let names: Vec<String> = std::iter::once(long)
            .chain((0..30).map(|n| format!("Wood{n}")))
            .collect();
        let reply = json!({
            "priorities": [{"property": "hardness", "weight": 1}],
            "avoid": names,
        })
        .to_string();

        let requirements = Requirements::parse(&reply).unwrap();

        assert_eq!(requirements.avoid.len(), MAX_AVOID);
        assert_eq!(requirements.avoid[0], "wood0");
    }

    #[test]
    fn the_summary_and_language_are_cut_to_a_sane_length() {
        let reply = json!({
            "summary": "s".repeat(1000),
            "language": "l".repeat(1000),
            "priorities": [{"property": "hardness", "weight": 1}],
        })
        .to_string();

        let requirements = Requirements::parse(&reply).unwrap();

        assert_eq!(requirements.summary.chars().count(), MAX_SUMMARY_CHARS);
        assert_eq!(requirements.language.chars().count(), MAX_LANGUAGE_CHARS);
    }

    #[test]
    fn unusable_replies_are_understanding_errors() {
        for reply in [
            "",
            "I cannot help with that",
            "} backwards {",
            "{not json}",
            r#"{"priorities": [{"property": "beauty", "weight": 2}]}"#,
            r#"{"priorities": [{"property": "hardness"}]}"#,
            r#"{"summary": "x"}"#,
            r#"{"priorities": []}"#,
            r#"{"priorities": [{"property": "hardness", "weight": 0}]}"#,
        ] {
            let error = Requirements::parse(reply).unwrap_err();

            assert!(matches!(error, AskError::Understanding(_)), "{reply}");
            assert!(error.to_string().contains("unusable"));
        }
    }

    #[test]
    fn the_schema_names_every_property_the_ranking_knows() {
        let schema = schema();

        let allowed =
            &schema["properties"]["priorities"]["items"]["properties"]["property"]["enum"];

        assert_eq!(allowed.as_array().unwrap().len(), Property::ALL.len());
        for property in Property::ALL {
            assert!(allowed.to_string().contains(property.as_str()));
            assert!(INSTRUCTIONS.contains(property.as_str()));
        }
    }
}
