use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use scraper::ElementRef;
use zylotek_domain::woods::value_objects::{
    Attributes, BotanicalFamily, CommonNames, KilnDrying, ScientificName, WoodDescription, WoodName,
};

use crate::parsers::{helpers, selectors};

/// Identification and descriptive data from the top of a species page.
#[derive(Debug, Clone)]
pub(crate) struct BasicInformation {
    pub name: WoodName,
    pub scientific_name: ScientificName,
    pub family: BotanicalFamily,
    pub collection_location: String,
    pub common_names: CommonNames,
    pub description: WoodDescription,
    pub kiln_drying: KilnDrying,
    pub images: ImageSources,
}

/// Image `src` attributes as found on the page, possibly relative to it.
#[derive(Debug, Clone)]
pub(crate) struct ImageSources {
    pub main: String,
    pub tree: Option<String>,
    pub log: Option<String>,
    pub bark: Option<String>,
    pub tangential_cut: Option<String>,
    pub cross_section: Option<String>,
    pub radial_cut: Option<String>,
    pub wood: Option<String>,
}

pub(crate) fn parse(container: ElementRef<'_>) -> Result<BasicInformation> {
    let name_selector = selectors::query("h4")?;
    let name = container
        .select(&name_selector)
        .next()
        .map(selectors::text)
        .ok_or_else(|| anyhow!("species name not found"))?;

    // Gallery photos sit inside a link; only the page's own picture is the main image.
    let image_url_selector = selectors::query("*:not(a) > img")?;
    let image_url = container
        .select(&image_url_selector)
        .next()
        .ok_or_else(|| anyhow!("image url not found"))?
        .attr("src")
        .ok_or_else(|| anyhow!("the main image has no src"))?
        .to_string();

    let p_selector = selectors::query("p")?;
    let span_selector = selectors::query("span")?;

    let mut label_and_texts = HashMap::new();
    for p_element in container.select(&p_selector) {
        let inner_text = p_element.text().collect::<String>();
        let label = p_element
            .select(&span_selector)
            .next()
            .map(|span| span.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let text = inner_text
            .strip_prefix(&label)
            .unwrap_or(&inner_text)
            .trim()
            .to_string();

        let attr_name = label.replace(':', "").to_lowercase();
        if attr_name.is_empty() {
            continue;
        }

        label_and_texts.insert(attr_name, text);
    }

    let mut images = HashMap::new();
    let img_selector = selectors::query(r#"div > a > img"#)?;
    for img_element in container.select(&img_selector) {
        // The caption is the first <p> of the <div> holding the link that wraps the image.
        let attr_name = img_element
            .parent()
            .and_then(|link| link.parent())
            .and_then(ElementRef::wrap)
            .and_then(|element| element.select(&p_selector).next())
            .map(|p| p.text().collect::<String>().trim().to_lowercase())
            .unwrap_or_default();
        if attr_name.is_empty() {
            continue;
        }
        img_element
            .attr("src")
            .map(|src| images.insert(attr_name, src.to_string()));
    }

    let scientific_name = label_and_texts
        .remove(html::SCIENTIFIC_NAME_LABEL)
        .with_context(|| "scientific name not found")?;

    let family = label_and_texts
        .remove(html::FAMILY_LABEL)
        .with_context(|| "family not found")?;

    let collection_location = label_and_texts
        .remove(html::COLLECTION_LOCATION_LABEL)
        .with_context(|| "collection location not found")?;

    let common_names = label_and_texts
        .remove(html::COMMON_NAMES_LABEL)
        .map(|text| CommonNames::new(text.split(',')))
        .unwrap_or_default();

    let description = WoodDescription {
        tree: labelled_pairs(label_and_texts.remove(html::TREE)),
        general_characteristics: labelled_pairs(
            label_and_texts.remove(html::GENERAL_CHARACTERISTICS_LABEL),
        ),
        end_uses: label_and_texts.remove(html::END_USES_LABEL),
        preservation: label_and_texts.remove(html::PRESERVATION_LABEL),
        workability_notes: label_and_texts.remove(html::WORKABILITY_LABEL),
        heartwood_color_classification: label_and_texts
            .remove(html::HEARTWOOD_COLOR_CLASSIFICATION_LABEL),
    };

    let kiln_drying = KilnDrying {
        schedule: labelled_pairs(label_and_texts.remove(html::KILN_DRYING_LABEL)),
        duration: helpers::measurement(
            label_and_texts
                .remove(html::KILN_DRYING_DAYS_LABEL)
                .as_ref(),
        )?,
    };

    let image_sources = ImageSources {
        main: image_url,
        tree: images.remove(html::TREE_IMAGE_LABEL),
        log: images.remove(html::LOG_IMAGE_LABEL),
        bark: images.remove(html::BARK_IMAGE_LABEL),
        tangential_cut: images.remove(html::TANGENTIAL_CUT_IMAGE_LABEL),
        cross_section: images.remove(html::CROSS_SECTION_IMAGE_LABEL),
        radial_cut: images.remove(html::RADIAL_CUT_IMAGE_LABEL),
        wood: images.remove(html::WOOD_IMAGE_LABEL),
    };

    if !label_and_texts.is_empty() || !images.is_empty() {
        log::warn!(
            "Attributes were not processed for {scientific_name}: {label_and_texts:?}, {images:?}"
        );
    }

    Ok(BasicInformation {
        name: WoodName::new(name)?,
        scientific_name: ScientificName::new(scientific_name)?,
        family: BotanicalFamily::new(family)?,
        collection_location,
        common_names,
        description,
        kiln_drying,
        images: image_sources,
    })
}

/// Parses `"label: value; label: value"` text into attributes.
fn labelled_pairs(text: Option<String>) -> Attributes {
    let Some(text) = text else {
        return Attributes::default();
    };

    Attributes::new(text.split(';').map(|pair| {
        let mut parts = pair.splitn(2, ':');
        (
            parts.next().unwrap_or_default().to_owned(),
            parts.next().unwrap_or_default().to_owned(),
        )
    }))
}

mod html {
    pub const SCIENTIFIC_NAME_LABEL: &str = "nome científico";
    pub const FAMILY_LABEL: &str = "família";
    pub const COLLECTION_LOCATION_LABEL: &str = "local de coleta";
    pub const TREE: &str = "árvore";
    pub const COMMON_NAMES_LABEL: &str = "nomes comuns";
    pub const GENERAL_CHARACTERISTICS_LABEL: &str = "características gerais";
    pub const END_USES_LABEL: &str = "uso final";
    pub const KILN_DRYING_LABEL: &str = "secagem em estufa";
    pub const PRESERVATION_LABEL: &str = "preservação";
    pub const WORKABILITY_LABEL: &str = "trabalhabilidade";
    pub const KILN_DRYING_DAYS_LABEL: &str = "tempo de secagem (dias)";
    pub const HEARTWOOD_COLOR_CLASSIFICATION_LABEL: &str = "classificação da cor do cerne";
    pub const TREE_IMAGE_LABEL: &str = "árvore";
    pub const LOG_IMAGE_LABEL: &str = "tora";
    pub const BARK_IMAGE_LABEL: &str = "casca";
    pub const TANGENTIAL_CUT_IMAGE_LABEL: &str = "corte tangencial";
    pub const CROSS_SECTION_IMAGE_LABEL: &str = "seção transversal";
    pub const RADIAL_CUT_IMAGE_LABEL: &str = "corte radial";
    pub const WOOD_IMAGE_LABEL: &str = "madeira";
}
