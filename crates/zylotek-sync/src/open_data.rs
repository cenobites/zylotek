//! Parser for the LPF open-data export (`dados_abertos_banco_de_dados_de_madeiras_brasileiras`).
//!
//! The file is `;`-separated UTF-8 with CRLF line endings, a decimal comma and a dot for
//! thousands. Each row is one test lot — a species collected at one place — so the same
//! scientific name can appear on several rows. A blank cell is a single space, and
//! measurements are in SI units, whereas the domain stores kgf-based units.

use std::collections::HashMap;

use anyhow::{Context, Result, bail, ensure};
use csv::{ReaderBuilder, StringRecord, Trim};
use zylotek_domain::woods::value_objects::*;

use crate::ScrapedWood;

/// The dataset's landing page, used as the source of every imported wood.
pub const DATASET_URL: &str = "https://lpf.florestal.gov.br/pt-br/bd-madeiras-brasileiras";

/// 1 kgf = 9.80665 N, so a stress of 1 MPa (= 100 N/cm²) is 100 / 9.80665 kgf/cm².
const KGF_PER_CM2_PER_MPA: f64 = 100.0 / 9.80665;
/// The modulus-of-elasticity columns are documented as MPa but hold GPa (values of 4 to 19).
const KGF_PER_CM2_PER_GPA: f64 = 1_000.0 * KGF_PER_CM2_PER_MPA;
/// Forces (N) and cleavage strength (N/cm) become kgf and kgf/cm.
const KGF_PER_N: f64 = 1.0 / 9.80665;

const ID: &str = "id_especie";
const SCIENTIFIC_NAME: &str = "nome_cientifico";
const POPULAR_NAME: &str = "nome_popular_1";
const REQUIRED: [&str; 3] = [ID, SCIENTIFIC_NAME, POPULAR_NAME];

/// Whether a literal `0` means "not measured" (the dataset does this for densities and
/// strengths, where zero is physically impossible).
const ZERO_IS_MISSING: bool = true;

const TREE: [(&str, &str); 3] = [
    ("altura comercial (m)", "arvore_altura_comercial_m"),
    ("dap (cm)", "arvore_dap_cm"),
    ("das (cm)", "arvore_das_cm"),
];

const GENERAL_CHARACTERISTICS: [(&str, &str); 19] = [
    ("autor do nome científico", "autor_nome_cientifico"),
    ("textura", "textura"),
    ("grã", "gra"),
    ("brilho", "brilho"),
    ("cheiro", "cheiro"),
    ("sabor", "sabor"),
    ("figura tangencial", "figura_tangencial"),
    ("figura radial", "figura_radial"),
    ("cerne e alburno", "cerne_alburno"),
    ("camadas de crescimento", "camadas_de_crescimento"),
    ("classificação da densidade", "densidade_classificacao"),
    (
        "classificação da compressão paralela",
        "compressaofco_classificacao",
    ),
    (
        "classificação da compressão perpendicular",
        "compressaofc90_classificacao",
    ),
    (
        "classificação da contração radial",
        "contracao_radial_classificacao",
    ),
    (
        "classificação da contração volumétrica",
        "contracao_volumetrica_classificacao",
    ),
    (
        "classificação da dureza paralela",
        "dureza_paralela_classificacao",
    ),
    (
        "classificação da dureza transversal",
        "dureza_tranversal_classificacao",
    ),
    (
        "classificação do módulo de elasticidade",
        "modulo_elasticidade_classificacao",
    ),
    (
        "classificação do cisalhamento",
        "cisalhamento_classificacao",
    ),
];

const KILN_SCHEDULE: [(&str, &str); 5] = [
    ("programa utilizado", "secagem_programa_utilizado"),
    ("umidade inicial (%)", "secagem_umidade_inicial"),
    ("umidade final (%)", "secagem_umidade_final"),
    ("número de amostras", "secagem_número_amostras"),
    (
        "classificação do tempo de secagem",
        "classificacao_tempo_secagem",
    ),
];

/// What a file contained: the woods that parsed, and the rows that did not.
#[derive(Debug, Default)]
pub struct OpenData {
    pub woods: Vec<ScrapedWood>,
    pub rejected: Vec<RejectedRow>,
}

/// A row that could not become a wood, so one bad row never hides the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedRow {
    /// Line in the file (a quoted newline makes a row span several).
    pub line: u64,
    /// The row's `id_especie`, when it has one.
    pub id: Option<String>,
    pub reason: String,
}

/// Parses the open-data CSV. Fails only when the file as a whole is unusable (no header,
/// or required columns missing); row problems are reported in [`OpenData::rejected`].
pub fn parse_open_data(input: &[u8]) -> Result<OpenData> {
    let input = input.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(input);
    let mut reader = ReaderBuilder::new()
        .delimiter(b';')
        .trim(Trim::All)
        .flexible(true)
        .from_reader(input);
    let columns = Columns::new(
        reader
            .headers()
            .context("the file has no readable header")?,
    )?;

    let mut data = OpenData::default();
    for record in reader.records() {
        match record {
            Ok(record) => match columns.wood(&record) {
                Ok(wood) => data.woods.push(wood),
                Err(error) => data.rejected.push(RejectedRow {
                    line: file_line(record.position()),
                    id: columns.get(&record, ID).map(str::to_owned),
                    reason: format!("{error:#}"),
                }),
            },
            Err(error) => data.rejected.push(RejectedRow {
                line: file_line(error.position()),
                id: None,
                reason: error.to_string(),
            }),
        }
    }

    Ok(data)
}

/// The csv crate numbers the records that follow the header from 1, so the line in the
/// file is one more.
fn file_line(position: Option<&csv::Position>) -> u64 {
    position.map_or(0, |position| position.line() + 1)
}

/// Column positions by lower-cased header name.
struct Columns(HashMap<String, usize>);

impl Columns {
    fn new(headers: &StringRecord) -> Result<Self> {
        let positions: HashMap<String, usize> = headers
            .iter()
            .enumerate()
            .map(|(position, header)| (header.to_lowercase(), position))
            .collect();
        let missing: Vec<&str> = REQUIRED
            .into_iter()
            .filter(|column| !positions.contains_key(*column))
            .collect();
        if !missing.is_empty() {
            bail!(
                "the file lacks the required columns: {}",
                missing.join(", ")
            );
        }

        Ok(Self(positions))
    }

    /// The cell's text, or `None` when the column is absent or the cell blank.
    fn get<'a>(&self, record: &'a StringRecord, column: &str) -> Option<&'a str> {
        let value = record.get(*self.0.get(column)?)?.trim();

        (!value.is_empty() && value != "-").then_some(value)
    }

    fn required<'a>(&self, record: &'a StringRecord, column: &str) -> Result<&'a str> {
        self.get(record, column)
            .with_context(|| format!("{column} is empty"))
    }

    fn number(&self, record: &StringRecord, column: &str) -> Result<Option<f64>> {
        self.get(record, column)
            .map(|text| parse_number(text).with_context(|| format!("{column}: {text:?}")))
            .transpose()
    }

    /// A measurement converted with `factor` into the domain's unit.
    fn measurement<M: Measurement>(
        &self,
        record: &StringRecord,
        column: &str,
        factor: f64,
        zero_is_missing: bool,
    ) -> Result<Option<M>> {
        let Some(value) = self.number(record, column)? else {
            return Ok(None);
        };
        if zero_is_missing && value == 0.0 {
            return Ok(None);
        }

        M::new(round_to_hundredths(value * factor))
            .map(Some)
            .with_context(|| column.to_owned())
    }

    /// A mechanical strength: like [`Self::measurement`], where a zero means "not measured".
    fn strength<M: Measurement>(
        &self,
        record: &StringRecord,
        column: &str,
        factor: f64,
    ) -> Result<Option<M>> {
        self.measurement(record, column, factor, ZERO_IS_MISSING)
    }

    fn attributes(&self, record: &StringRecord, specs: &[(&str, &str)]) -> Vec<(String, String)> {
        specs
            .iter()
            .filter_map(|(label, column)| {
                Some(((*label).to_owned(), self.get(record, column)?.to_owned()))
            })
            .collect()
    }

    fn wood(&self, record: &StringRecord) -> Result<ScrapedWood> {
        let species_id = self
            .required(record, ID)?
            .parse::<u64>()
            .context("id_especie is not an integer")?;
        let name = WoodName::new(sentence_case(self.required(record, POPULAR_NAME)?))?;

        // "Handroanthus serratifolius = Tabebuia serratifolia": accepted name = synonym.
        let full_name = self.required(record, SCIENTIFIC_NAME)?;
        let (accepted, synonym) = match full_name.split_once('=') {
            Some((accepted, synonym)) => (accepted.trim(), Some(synonym.trim())),
            None => (full_name, None),
        };
        let scientific_name = ScientificName::new(accepted)?;

        let mut general = self.attributes(record, &GENERAL_CHARACTERISTICS);
        general.extend(synonym.map(|synonym| ("sinonímia".to_owned(), synonym.to_owned())));

        let collection_location = match (
            self.get(record, "municipio_coleta"),
            self.get(record, "estado_coleta"),
        ) {
            (Some(city), Some(state)) => Some(format!("{city}, {state}")),
            (Some(place), None) | (None, Some(place)) => Some(place.to_owned()),
            (None, None) => None,
        };

        let details = WoodDetails {
            family: self
                .get(record, "familia")
                .map(BotanicalFamily::new)
                .transpose()?,
            collection_location,
            common_names: CommonNames::default(),
            source: Some(SourceReference {
                species_id,
                page_url: WebUrl::new(DATASET_URL)?,
            }),
            description: WoodDescription {
                tree: Attributes::new(self.attributes(record, &TREE)),
                general_characteristics: Attributes::new(general),
                heartwood_color_classification: self
                    .get(record, "cor_cerne_classificacao")
                    .map(str::to_owned),
                ..Default::default()
            },
            kiln_drying: KilnDrying {
                schedule: Attributes::new(self.attributes(record, &KILN_SCHEDULE)),
                duration: self.measurement(record, "secagem_duracao_dias", 1.0, ZERO_IS_MISSING)?,
            },
            images: WoodImages::default(),
            physical_properties: self.physical_properties(record)?,
            mechanical_properties: MechanicalProperties {
                green: self.mechanical_properties(record, "verde")?,
                dry: self.mechanical_properties(record, "seca")?,
            },
            workabilities: Workabilities::default(),
        };

        Ok(ScrapedWood {
            name,
            scientific_name,
            details,
        })
    }

    fn physical_properties(&self, record: &StringRecord) -> Result<PhysicalProperties> {
        let density = |column| self.measurement(record, column, 1.0, ZERO_IS_MISSING);
        let shrinkage = |column| self.measurement(record, column, 1.0, !ZERO_IS_MISSING);

        Ok(PhysicalProperties {
            density: Density {
                dry: density("densidade_seca")?,
                green: density("densidade_verde")?,
                basic: density("densidade_basica")?,
                apparent: density("densidade_aparente")?,
            },
            shrinkage: Shrinkage {
                tangential: shrinkage("contracao_tangencial")?,
                radial: shrinkage("contracao_radial")?,
                volumetric: shrinkage("contracao_volumetrica")?,
            },
        })
    }

    /// `condition` is the column suffix: `verde` (green) or `seca` (dry).
    ///
    /// Janka hardness and nail extraction have a "parallel" and a "transversal" or
    /// "perpendicular" column; they fill the domain's `parallel_to_grain` and `end_grain`
    /// fields respectively.
    fn mechanical_properties(
        &self,
        record: &StringRecord,
        condition: &str,
    ) -> Result<MechanicalPropertiesByCondition> {
        let suffixed = |name: &str| format!("{name}_{condition}");

        Ok(MechanicalPropertiesByCondition {
            static_bending: StaticBending {
                modulus_of_rupture: self.strength(
                    record,
                    &format!("flexao_{condition}_mor"),
                    KGF_PER_CM2_PER_MPA,
                )?,
                modulus_of_elasticity: self.strength(
                    record,
                    &format!("flexao_{condition}_moe"),
                    KGF_PER_CM2_PER_GPA,
                )?,
            },
            compression: Compression {
                parallel_to_grain: self.strength(
                    record,
                    &suffixed("compressao_paralela"),
                    KGF_PER_CM2_PER_MPA,
                )?,
                perpendicular_to_grain: self.strength(
                    record,
                    &suffixed("compressao_perpendicular"),
                    KGF_PER_CM2_PER_MPA,
                )?,
            },
            janka_hardness: JankaHardness {
                parallel_to_grain: self.strength(
                    record,
                    &suffixed("dureza_janka_paralela"),
                    KGF_PER_N,
                )?,
                end_grain: self.strength(
                    record,
                    &suffixed("dureza_janka_transversal"),
                    KGF_PER_N,
                )?,
            },
            tension_perpendicular_to_grain: self.strength(
                record,
                &suffixed("tração_perpendicular"),
                KGF_PER_CM2_PER_MPA,
            )?,
            cleavage_strength: self.strength(record, &suffixed("fendilhamento"), KGF_PER_N)?,
            shear_parallel_to_grain: self.strength(
                record,
                &suffixed("cisalhamento"),
                KGF_PER_CM2_PER_MPA,
            )?,
            nail_extraction: NailExtraction {
                parallel_to_grain: self.strength(
                    record,
                    &suffixed("extração_pregos_paralela"),
                    KGF_PER_N,
                )?,
                end_grain: self.strength(
                    record,
                    &suffixed("extração_pregos_perpendicular"),
                    KGF_PER_N,
                )?,
            },
        })
    }
}

/// Brazilian number: `1.059,12` is 1059.12 and `0,5` is 0.5. Without a comma the dot is
/// a decimal point (the dataset never uses a bare dot for thousands).
fn parse_number(text: &str) -> Result<f64> {
    let normalized = if text.contains(',') {
        text.replace('.', "").replace(',', ".")
    } else {
        text.to_owned()
    };
    let value: f64 = normalized.parse().context("not a number")?;
    ensure!(value.is_finite(), "not a finite number");

    Ok(value)
}

fn round_to_hundredths(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// "MUIRACATIARA-RAJADA" becomes "Muiracatiara-rajada", as the web site writes it.
fn sentence_case(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut chars = lower.chars();

    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: [&str; 67] = [
        "id_especie",
        "nome_cientifico",
        "nome_popular_1",
        "genero",
        "especie",
        "familia",
        "autor_nome_cientifico",
        "municipio_coleta",
        "estado_coleta",
        "arvore_altura_comercial_m",
        "arvore_dap_cm",
        "arvore_das_cm",
        "densidade_basica",
        "densidade_seca",
        "densidade_verde",
        "densidade_aparente",
        "contracao_tangencial",
        "contracao_radial",
        "contracao_volumetrica",
        "relacao_tangencial_radial",
        "flexao_verde_moe",
        "flexao_verde_mor",
        "flexao_seca_moe",
        "flexao_seca_mor",
        "compressao_paralela_verde",
        "compressao_paralela_seca",
        "compressao_perpendicular_verde",
        "compressao_perpendicular_seca",
        "dureza_janka_paralela_verde",
        "dureza_janka_paralela_seca",
        "dureza_janka_transversal_verde",
        "dureza_janka_transversal_seca",
        "tração_perpendicular_verde",
        "tração_perpendicular_seca",
        "fendilhamento_verde",
        "fendilhamento_seca",
        "cisalhamento_verde",
        "cisalhamento_seca",
        "extração_pregos_paralela_verde",
        "extração_pregos_paralela_seca",
        "extração_pregos_perpendicular_verde",
        "extração_pregos_perpendicular_seca",
        "cor_cerne_classificacao",
        "textura",
        "gra",
        "brilho",
        "cheiro",
        "sabor",
        "figura_tangencial",
        "figura_radial",
        "cerne_alburno",
        "camadas_de_crescimento",
        "secagem_duracao_dias",
        "secagem_umidade_inicial",
        "secagem_umidade_final",
        "secagem_programa_utilizado",
        "secagem_número_amostras",
        "densidade_classificacao",
        "compressaofco_classificacao",
        "compressaofc90_classificacao",
        "contracao_radial_classificacao",
        "contracao_volumetrica_classificacao",
        "dureza_paralela_classificacao",
        "dureza_tranversal_classificacao",
        "classificacao_tempo_secagem",
        "Modulo_elasticidade_classificacao",
        "cisalhamento_classificacao",
    ];

    /// A file in the dataset's shape: every column, CRLF, and a space for each blank cell.
    fn file(rows: &[&[(&str, &str)]]) -> Vec<u8> {
        let mut text = COLUMNS.join(";") + "\r\n";
        for overrides in rows {
            let cells: Vec<&str> = COLUMNS
                .iter()
                .map(|column| {
                    overrides
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(column))
                        .map_or(" ", |(_, value)| *value)
                })
                .collect();
            text += &(cells.join(";") + "\r\n");
        }
        text.into_bytes()
    }

    const BASE: [(&str, &str); 3] = [
        ("id_especie", "1"),
        ("nome_cientifico", "Genus species"),
        ("nome_popular_1", "POPULAR"),
    ];

    /// The base row with `extra` cells replacing or adding to it.
    fn row<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        BASE.iter()
            .filter(|(name, _)| !extra.iter().any(|(overridden, _)| overridden == name))
            .chain(extra)
            .copied()
            .collect()
    }

    fn parse_one(extra: &[(&str, &str)]) -> ScrapedWood {
        let data = parse_open_data(&file(&[&row(extra)])).unwrap();
        assert!(data.rejected.is_empty(), "{:?}", data.rejected);
        data.woods.into_iter().next().unwrap()
    }

    fn reject_one(extra: &[(&str, &str)]) -> RejectedRow {
        let data = parse_open_data(&file(&[&row(extra)])).unwrap();
        assert!(data.woods.is_empty());
        data.rejected.into_iter().next().unwrap()
    }

    fn value<M: Measurement>(measurement: Option<M>) -> Option<f64> {
        measurement.map(|m| m.value())
    }

    #[test]
    fn a_minimal_row_becomes_a_wood_with_empty_details() {
        let wood = parse_one(&[]);

        assert_eq!(wood.name.as_str(), "Popular");
        assert_eq!(wood.scientific_name.as_str(), "Genus species");
        let details = wood.details;
        assert_eq!(details.family, None);
        assert_eq!(details.collection_location, None);
        assert!(details.common_names.is_empty());
        assert!(details.description.tree.is_empty());
        assert!(details.description.general_characteristics.is_empty());
        assert_eq!(details.kiln_drying, KilnDrying::default());
        assert_eq!(details.physical_properties, PhysicalProperties::default());
        assert_eq!(
            details.mechanical_properties,
            MechanicalProperties::default()
        );
        assert_eq!(details.workabilities, Workabilities::default());
        let source = details.source.unwrap();
        assert_eq!(
            (source.species_id, source.page_url.as_str()),
            (1, DATASET_URL)
        );
    }

    #[test]
    fn identification_and_descriptive_columns_are_mapped() {
        let wood = parse_one(&[
            ("familia", "Anacardiaceae"),
            ("autor_nome_cientifico", "Ducke"),
            ("municipio_coleta", "Belém"),
            ("estado_coleta", "PA"),
            ("arvore_altura_comercial_m", "9"),
            ("arvore_dap_cm", "60,5"),
            ("cor_cerne_classificacao", "marrom"),
            ("textura", "média"),
            ("gra", "ondulada"),
            ("figura_radial", "destacada"),
            ("densidade_classificacao", "Pesada"),
            ("Modulo_elasticidade_classificacao", "Alto"),
            ("secagem_programa_utilizado", "suave"),
            ("secagem_umidade_inicial", "58,2"),
            ("classificacao_tempo_secagem", "Rápida - 4 a 8"),
            ("secagem_duracao_dias", "6"),
        ]);
        let details = wood.details;

        assert_eq!(details.family.unwrap().as_str(), "Anacardiaceae");
        assert_eq!(details.collection_location.as_deref(), Some("Belém, PA"));
        assert_eq!(
            details.description.tree.get("altura comercial (m)"),
            Some("9")
        );
        assert_eq!(details.description.tree.get("dap (cm)"), Some("60,5"));
        assert_eq!(details.description.tree.get("das (cm)"), None);
        assert_eq!(
            details
                .description
                .heartwood_color_classification
                .as_deref(),
            Some("marrom")
        );
        let general = &details.description.general_characteristics;
        assert_eq!(general.get("autor do nome científico"), Some("Ducke"));
        assert_eq!(general.get("textura"), Some("média"));
        assert_eq!(general.get("grã"), Some("ondulada"));
        assert_eq!(general.get("figura radial"), Some("destacada"));
        assert_eq!(general.get("classificação da densidade"), Some("Pesada"));
        assert_eq!(
            general.get("classificação do módulo de elasticidade"),
            Some("Alto")
        );
        assert_eq!(
            details.kiln_drying.schedule.get("programa utilizado"),
            Some("suave")
        );
        assert_eq!(
            details.kiln_drying.schedule.get("umidade inicial (%)"),
            Some("58,2")
        );
        assert_eq!(
            details
                .kiln_drying
                .schedule
                .get("classificação do tempo de secagem"),
            Some("Rápida - 4 a 8")
        );
        assert_eq!(value(details.kiln_drying.duration), Some(6.0));
    }

    #[test]
    fn the_collection_location_uses_whatever_place_is_known() {
        let location =
            |extra: &[(&'static str, &'static str)]| parse_one(extra).details.collection_location;

        assert_eq!(
            location(&[("municipio_coleta", "Santarém")]).as_deref(),
            Some("Santarém")
        );
        assert_eq!(location(&[("estado_coleta", "AM")]).as_deref(), Some("AM"));
        assert_eq!(location(&[]), None);
    }

    #[test]
    fn a_synonym_after_an_equals_sign_is_kept_as_an_attribute() {
        let wood = parse_one(&[(
            "nome_cientifico",
            "Handroanthus serratifolius = Tabebuia serratifolia",
        )]);

        assert_eq!(wood.scientific_name.as_str(), "Handroanthus serratifolius");
        assert_eq!(
            wood.details
                .description
                .general_characteristics
                .get("sinonímia"),
            Some("Tabebuia serratifolia")
        );
    }

    #[test]
    fn physical_properties_are_read_as_they_are() {
        let physical = parse_one(&[
            ("densidade_seca", "0,84"),
            ("densidade_verde", "1,17"),
            ("densidade_basica", "0,75"),
            ("densidade_aparente", "0,9"),
            ("contracao_tangencial", "7,2"),
            ("contracao_radial", "4,1"),
            ("contracao_volumetrica", "11"),
        ])
        .details
        .physical_properties;

        assert_eq!(value(physical.density.dry), Some(0.84));
        assert_eq!(value(physical.density.green), Some(1.17));
        assert_eq!(value(physical.density.basic), Some(0.75));
        assert_eq!(value(physical.density.apparent), Some(0.9));
        assert_eq!(value(physical.shrinkage.tangential), Some(7.2));
        assert_eq!(value(physical.shrinkage.radial), Some(4.1));
        assert_eq!(value(physical.shrinkage.volumetric), Some(11.0));
    }

    /// Row 21 of the real file, with the SI → kgf conversions worked out independently.
    #[test]
    fn mechanical_properties_are_converted_from_si_to_kgf() {
        let wood = parse_one(&[
            ("flexao_verde_moe", "11,28"),
            ("flexao_verde_mor", "100,62"),
            ("flexao_seca_moe", "12,94"),
            ("flexao_seca_mor", "145,63"),
            ("compressao_paralela_verde", "52,07"),
            ("compressao_perpendicular_seca", "14,61"),
            ("dureza_janka_paralela_verde", "6590,09"),
            ("dureza_janka_transversal_seca", "8659,29"),
            ("tração_perpendicular_verde", "6,77"),
            ("fendilhamento_seca", "568,79"),
            ("cisalhamento_seca", "11,77"),
            ("extração_pregos_paralela_verde", "1.059,12"),
            ("extração_pregos_perpendicular_seca", "882,6"),
        ]);
        let green = &wood.details.mechanical_properties.green;
        let dry = &wood.details.mechanical_properties.dry;
        let close = |actual: Option<f64>, expected: f64| {
            let actual = actual.expect("a value");
            assert!((actual - expected).abs() < 0.011, "{actual} != {expected}");
        };

        close(value(green.static_bending.modulus_of_rupture), 1026.04);
        close(
            value(green.static_bending.modulus_of_elasticity),
            115_023.99,
        );
        close(value(dry.static_bending.modulus_of_elasticity), 131_951.28);
        close(value(dry.static_bending.modulus_of_rupture), 1485.01);
        close(value(green.compression.parallel_to_grain), 530.97);
        close(value(dry.compression.perpendicular_to_grain), 148.98);
        close(value(green.janka_hardness.parallel_to_grain), 672.0);
        close(value(dry.janka_hardness.end_grain), 883.0);
        close(value(green.tension_perpendicular_to_grain), 69.03);
        close(value(dry.cleavage_strength), 58.0);
        close(value(dry.shear_parallel_to_grain), 120.02);
        close(value(green.nail_extraction.parallel_to_grain), 108.0);
        close(value(dry.nail_extraction.end_grain), 90.0);
        assert_eq!(green.cleavage_strength, None, "blank stays unknown");
    }

    #[test]
    fn a_zero_density_or_strength_means_not_measured_but_zero_shrinkage_is_kept() {
        let wood = parse_one(&[
            ("densidade_basica", "0"),
            ("fendilhamento_verde", "0"),
            ("secagem_duracao_dias", "0"),
            ("contracao_radial", "0"),
        ]);
        let details = wood.details;

        assert_eq!(details.physical_properties.density.basic, None);
        assert_eq!(details.mechanical_properties.green.cleavage_strength, None);
        assert_eq!(details.kiln_drying.duration, None);
        assert_eq!(
            value(details.physical_properties.shrinkage.radial),
            Some(0.0)
        );
    }

    #[test]
    fn numbers_follow_the_brazilian_format() {
        for (text, expected) in [
            ("0,5", 0.5),
            ("9", 9.0),
            ("1.059,12", 1059.12),
            ("1.235.000,5", 1_235_000.5),
            ("2.5", 2.5),
            ("-3,5", -3.5),
        ] {
            assert_eq!(parse_number(text).unwrap(), expected, "{text}");
        }
        for text in ["abc", "1,2,3", "NaN", "inf", "-inf", ""] {
            assert!(parse_number(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn popular_names_are_written_in_sentence_case() {
        assert_eq!(sentence_case("MUIRACATIARA-RAJADA"), "Muiracatiara-rajada");
        assert_eq!(sentence_case("rosaDINHO"), "Rosadinho");
        assert_eq!(sentence_case("ÁRVORE DA CHUVA"), "Árvore da chuva");
        assert_eq!(sentence_case(""), "");
    }

    #[test]
    fn padded_cells_blank_markers_bom_and_short_rows_are_tolerated() {
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(
            "id_especie;nome_cientifico;nome_popular_1;familia;estado_coleta\r\n\
             7 ; Genus   species ;  JACAREÚBA ; Fabaceae ;PA \r\n\
             8;Other thing;ANDIROBA;-\r\n"
                .as_bytes(),
        );

        let data = parse_open_data(&bytes).unwrap();

        assert!(data.rejected.is_empty(), "{:?}", data.rejected);
        assert_eq!(data.woods.len(), 2);
        assert_eq!(data.woods[0].name.as_str(), "Jacareúba");
        assert_eq!(data.woods[0].scientific_name.as_str(), "Genus species");
        assert_eq!(
            data.woods[0].details.collection_location.as_deref(),
            Some("PA")
        );
        assert_eq!(
            data.woods[1].details.family, None,
            "'-' and missing cells are blank"
        );
        assert_eq!(data.woods[1].details.collection_location, None);
    }

    #[test]
    fn the_parser_keeps_every_lot_of_a_repeated_species() {
        let first = row(&[("id_especie", "1"), ("municipio_coleta", "Belém")]);
        let second = row(&[("id_especie", "2"), ("municipio_coleta", "Santarém")]);

        let data = parse_open_data(&file(&[&first, &second])).unwrap();

        assert_eq!(data.woods.len(), 2);
        assert_eq!(data.woods[0].scientific_name, data.woods[1].scientific_name);
    }

    #[test]
    fn bad_rows_are_rejected_with_their_line_and_id_while_the_rest_import() {
        let good = row(&[("id_especie", "1")]);
        let bad = row(&[("id_especie", "2"), ("nome_cientifico", "Solo")]);
        let also_good = row(&[("id_especie", "3"), ("nome_cientifico", "Other species")]);

        let data = parse_open_data(&file(&[&good, &bad, &also_good])).unwrap();

        assert_eq!(data.woods.len(), 2);
        assert_eq!(data.rejected.len(), 1);
        let rejected = &data.rejected[0];
        assert_eq!((rejected.line, rejected.id.as_deref()), (3, Some("2")));
        assert!(
            rejected.reason.contains("genus and a specific epithet"),
            "{}",
            rejected.reason
        );
    }

    #[test]
    fn each_kind_of_bad_value_is_rejected_with_a_reason() {
        let cases: [(&[(&'static str, &'static str)], &str); 9] = [
            (&[("id_especie", "abc")], "not an integer"),
            (&[("id_especie", " ")], "id_especie is empty"),
            (&[("nome_popular_1", " ")], "nome_popular_1 is empty"),
            (&[("nome_cientifico", " ")], "nome_cientifico is empty"),
            (&[("familia", "x")], "x"),
            (&[("densidade_seca", "abc")], "densidade_seca"),
            (&[("flexao_seca_mor", "-5")], "flexao_seca_mor"),
            (&[("contracao_radial", "101")], "contracao_radial"),
            (
                &[("dureza_janka_paralela_seca", "NaN")],
                "dureza_janka_paralela_seca",
            ),
        ];

        for (extra, reason) in cases {
            if extra == [("familia", "x")] {
                // A one-letter family is fine; only blanks are rejected, and those are None.
                assert!(
                    parse_open_data(&file(&[&row(extra)]))
                        .unwrap()
                        .rejected
                        .is_empty()
                );
                continue;
            }
            let rejected = reject_one(extra);

            assert!(
                rejected.reason.contains(reason),
                "{reason}: {}",
                rejected.reason
            );
        }
    }

    #[test]
    fn an_invalid_value_in_any_mechanical_column_rejects_the_row() {
        for condition in ["verde", "seca"] {
            let columns = [
                format!("flexao_{condition}_mor"),
                format!("flexao_{condition}_moe"),
                format!("compressao_paralela_{condition}"),
                format!("compressao_perpendicular_{condition}"),
                format!("dureza_janka_paralela_{condition}"),
                format!("dureza_janka_transversal_{condition}"),
                format!("tração_perpendicular_{condition}"),
                format!("fendilhamento_{condition}"),
                format!("cisalhamento_{condition}"),
                format!("extração_pregos_paralela_{condition}"),
                format!("extração_pregos_perpendicular_{condition}"),
            ];

            for column in &columns {
                let rejected = reject_one(&[(column.as_str(), "-1")]);

                assert!(
                    rejected.reason.contains(column.as_str()),
                    "{column}: {}",
                    rejected.reason
                );
            }
        }
    }

    #[test]
    fn invalid_utf8_rejects_the_row_not_the_file() {
        let mut bytes = file(&[&row(&[("id_especie", "1")])]);
        bytes.extend_from_slice(b"2;Genus other;NOME\xFF;");
        bytes.extend_from_slice(&b";".repeat(COLUMNS.len() - 4));
        bytes.extend_from_slice(b"\r\n");

        let data = parse_open_data(&bytes).unwrap();

        assert_eq!(data.woods.len(), 1);
        assert_eq!(data.rejected.len(), 1);
        assert_eq!(data.rejected[0].id, None);
        assert!(data.rejected[0].reason.contains("UTF-8"));
    }

    #[test]
    fn a_file_without_the_required_columns_is_rejected_as_a_whole() {
        for input in [
            "",
            "a;b;c\r\n1;2;3\r\n",
            "id_especie;nome_cientifico\r\n1;x y\r\n",
        ] {
            let error = parse_open_data(input.as_bytes()).unwrap_err();

            assert!(
                error.to_string().contains("lacks the required columns"),
                "{input:?}: {error}"
            );
        }
        let error = parse_open_data(b"id_especie;nome_cientifico\r\n").unwrap_err();
        assert!(error.to_string().contains("nome_popular_1"), "{error}");
    }

    #[test]
    fn a_header_only_file_has_no_woods() {
        let data = parse_open_data(&file(&[])).unwrap();

        assert!(data.woods.is_empty() && data.rejected.is_empty());
        assert!(
            format!(
                "{:?}",
                RejectedRow {
                    line: 1,
                    id: None,
                    reason: String::new()
                }
            )
            .contains("line")
        );
    }
}
