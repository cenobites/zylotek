//! Translations of the website's own text, and how a visitor's language is chosen.
//!
//! Each language is one `Strings` value, so a missing translation is a compile error. What
//! comes from the database (wood names, descriptions) or from the model is shown as it is.

use axum::http::{HeaderMap, header};

/// The languages the website speaks. English is the fallback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    PtBr,
}

pub const COOKIE: &str = "lang";
const ONE_YEAR_SECS: u32 = 365 * 24 * 60 * 60;

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::PtBr];

    /// The code used in `?lang=` and the cookie.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::PtBr => "pt-br",
        }
    }

    /// A language tag as browsers and people write it: `pt`, `pt-BR`, `pt_br`, `en-GB`.
    /// Any Portuguese is served the Brazilian translation, any English the English one.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let tag = tag.trim().to_ascii_lowercase().replace('_', "-");
        match tag.split('-').next() {
            Some("en") => Some(Lang::En),
            Some("pt") => Some(Lang::PtBr),
            _ => None,
        }
    }

    /// The best supported language in an `Accept-Language` value, honouring `q` weights.
    pub fn from_accept_language(value: &str) -> Option<Lang> {
        let mut ranked: Vec<(f32, Lang)> = value
            .split(',')
            .filter_map(|part| {
                let mut pieces = part.split(';');
                let lang = Lang::from_tag(pieces.next()?)?;
                let quality = pieces
                    .find_map(|p| p.trim().strip_prefix("q="))
                    .map_or(Some(1.0), |q| q.trim().parse::<f32>().ok())?;
                (quality > 0.0).then_some((quality, lang))
            })
            .collect();
        // Stable: among equal weights, the order the browser listed them in stands.
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));

        ranked.first().map(|(_, lang)| *lang)
    }

    /// The language for a request, and the one to remember when the visitor asked for it with
    /// `?lang=`. Order: that parameter, the cookie, `Accept-Language`, English.
    pub fn detect(query: Option<&str>, headers: &HeaderMap) -> (Lang, Option<Lang>) {
        let asked = form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .find(|(key, _)| key == "lang")
            .and_then(|(_, value)| Lang::from_tag(&value));
        if let Some(lang) = asked {
            return (lang, Some(lang));
        }

        let remembered = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .find_map(|pair| pair.trim().strip_prefix("lang="))
            .and_then(Lang::from_tag);
        let negotiated = || {
            headers
                .get(header::ACCEPT_LANGUAGE)
                .and_then(|value| value.to_str().ok())
                .and_then(Lang::from_accept_language)
        };

        (remembered.or_else(negotiated).unwrap_or_default(), None)
    }

    /// The `Set-Cookie` value that remembers this language.
    pub fn cookie(self) -> String {
        format!(
            "{COOKIE}={}; Path=/; Max-Age={ONE_YEAR_SECS}; SameSite=Lax",
            self.code()
        )
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::PtBr => &PT_BR,
        }
    }

    /// `1.5` or `1,5`.
    pub fn decimal(self, formatted: String) -> String {
        match self {
            Lang::En => formatted,
            Lang::PtBr => formatted.replace('.', ","),
        }
    }
}

/// All the fixed text of the pages.
pub struct Strings {
    pub lang: Lang,
    pub html_lang: &'static str,

    // Navigation and footer.
    pub nav_ask: &'static str,
    pub nav_browse: &'static str,
    pub footer: &'static str,

    // Home.
    pub home_title: &'static str,
    pub home_heading: &'static str,
    pub home_intro: &'static str,
    pub home_placeholder: &'static str,
    pub home_submit: &'static str,

    // The answer.
    pub answer_title: &'static str,
    pub answer_heading: &'static str,
    pub understood: &'static str,
    pub looked_at: &'static str,
    pub essential: &'static str,
    pub important: &'static str,
    pub no_matches: &'static str,
    pub match_word: &'static str,
    pub full_details: &'static str,
    pub compare_submit_range: &'static str,
    pub in_short: &'static str,
    pub ask_another: &'static str,
    pub explanation_cut_short: &'static str,

    // Browsing.
    pub browse_title: &'static str,
    pub browse_heading: &'static str,
    pub col_name: &'static str,
    pub col_scientific_name: &'static str,
    pub col_family: &'static str,
    pub col_hardness: &'static str,
    pub col_density: &'static str,
    pub compare_submit: &'static str,
    pub previous: &'static str,
    pub next: &'static str,

    // One wood.
    pub also_called: &'static str,
    pub key_figures: &'static str,
    pub typical_uses: &'static str,
    pub working_it: &'static str,
    pub durability: &'static str,
    pub general_characteristics: &'static str,
    pub the_tree: &'static str,
    pub source: &'static str,
    pub all_woods: &'static str,

    // Comparing.
    pub compare_title: &'static str,
    pub compare_hint: &'static str,

    // Errors.
    pub error_word: &'static str,
    pub back_to_start: &'static str,
    pub title_not_found: &'static str,
    pub title_bad_request: &'static str,
    pub title_unavailable: &'static str,
    pub title_internal: &'static str,
    pub message_internal: &'static str,
    pub that_page: &'static str,
    pub that_wood: &'static str,
    pub invalid_wood: &'static str,

    // The figures of a wood (also the table headings of the comparison).
    pub fact_family: &'static str,
    pub fact_density: &'static str,
    pub fact_shrinkage: &'static str,
    pub fact_hardness: &'static str,
    pub fact_bending: &'static str,
    pub fact_stiffness: &'static str,
    pub fact_workability: &'static str,

    // What the question was ranked on.
    pub prop_hardness: &'static str,
    pub prop_strength: &'static str,
    pub prop_stiffness: &'static str,
    pub prop_stability: &'static str,
    pub prop_lightness: &'static str,
    pub prop_workability: &'static str,
    pub shrinkage_suffix: &'static str,
    pub ease_suffix: &'static str,
}

impl Strings {
    pub fn question_length(&self, max: usize) -> String {
        match self.lang {
            Lang::En => format!("Please write between 1 and {max} characters."),
            Lang::PtBr => format!("Escreva entre 1 e {max} caracteres."),
        }
    }

    pub fn better_than(&self, percent: u32) -> String {
        match self.lang {
            Lang::En => format!("better than {percent}% of woods"),
            Lang::PtBr => format!("melhor que {percent}% das madeiras"),
        }
    }

    pub fn ranked_among(&self, count: usize) -> String {
        match self.lang {
            Lang::En => format!("Ranked among {count} woods."),
            Lang::PtBr => format!("Classificadas entre {count} madeiras."),
        }
    }

    pub fn showing(&self, first: usize, last: usize, total: u64) -> String {
        match self.lang {
            Lang::En => format!(
                "Showing {first}–{last} of {total}. Tick two to four woods to compare them."
            ),
            Lang::PtBr => format!(
                "Mostrando {first}–{last} de {total}. Marque de duas a quatro madeiras para compará-las."
            ),
        }
    }

    pub fn compare_label(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("Compare {name}"),
            Lang::PtBr => format!("Comparar {name}"),
        }
    }

    pub fn collected_in(&self, location: &str) -> String {
        match self.lang {
            Lang::En => format!("Collected in {location}."),
            Lang::PtBr => format!("Coletada em {location}."),
        }
    }

    pub fn could_not_find(&self, what: &str) -> String {
        match self.lang {
            Lang::En => format!("We could not find {what}."),
            Lang::PtBr => format!("Não encontramos {what}."),
        }
    }

    pub fn select_at_least(&self, min: usize) -> String {
        match self.lang {
            Lang::En => format!("Select at least {min} woods to compare."),
            Lang::PtBr => format!("Selecione pelo menos {min} madeiras para comparar."),
        }
    }

    pub fn compare_at_most(&self, max: usize) -> String {
        match self.lang {
            Lang::En => format!("Compare at most {max} woods at a time."),
            Lang::PtBr => format!("Compare no máximo {max} madeiras por vez."),
        }
    }

    pub fn unreachable(&self, service: &str) -> String {
        match self.lang {
            Lang::En => format!("We could not reach {service}. Please try again in a moment."),
            Lang::PtBr => {
                format!("Não conseguimos falar com {service}. Tente novamente em instantes.")
            }
        }
    }

    pub fn rejected(&self, service: &str, message: &str) -> String {
        match self.lang {
            Lang::En => format!("{service} could not use that: {message}"),
            Lang::PtBr => format!("{service} não conseguiu usar isso: {message}"),
        }
    }

    pub fn starting_up(&self, service: &str) -> String {
        match self.lang {
            Lang::En => format!("{service} is still starting up. Please try again in a moment."),
            Lang::PtBr => format!("{service} ainda está iniciando. Tente novamente em instantes."),
        }
    }

    pub fn had_a_problem(&self, service: &str) -> String {
        match self.lang {
            Lang::En => format!("{service} had a problem answering. Please try again."),
            Lang::PtBr => format!("{service} teve um problema ao responder. Tente novamente."),
        }
    }
}

pub static EN: Strings = Strings {
    lang: Lang::En,
    html_lang: "en",

    nav_ask: "Ask",
    nav_browse: "Browse woods",
    footer: "Data: Laboratório de Produtos Florestais / Serviço Florestal Brasileiro. Figures are indicative; test your own stock before building.",

    home_title: "Find the right wood",
    home_heading: "Which wood should I use?",
    home_intro: "Describe what you want to build, in your own words. We rank the Brazilian woods in our database for it.",
    home_placeholder: "I want to build a dining table for 12 seats using Japanese joinery",
    home_submit: "Find woods",

    answer_title: "Your woods",
    answer_heading: "Woods for your project",
    understood: "We understood:",
    looked_at: "What we looked at:",
    essential: "essential",
    important: "important",
    no_matches: "No wood in the database has the figures needed for this question.",
    match_word: "match",
    full_details: "Full details",
    compare_submit_range: "Compare selected (2 to 4)",
    in_short: "In short",
    ask_another: "Ask another question",
    explanation_cut_short: "The explanation was cut short.",

    browse_title: "Browse woods",
    browse_heading: "Brazilian woods",
    col_name: "Name",
    col_scientific_name: "Scientific name",
    col_family: "Family",
    col_hardness: "Hardness",
    col_density: "Density",
    compare_submit: "Compare selected",
    previous: "← Previous",
    next: "Next →",

    also_called: "also called",
    key_figures: "Key figures",
    typical_uses: "Typical uses",
    working_it: "Working it",
    durability: "Durability and preservation",
    general_characteristics: "General characteristics",
    the_tree: "The tree",
    source: "Source",
    all_woods: "← All woods",

    compare_title: "Compare woods",
    compare_hint: "Higher hardness, strength and stiffness resist more load. Lower shrinkage means a more stable board.",

    error_word: "Error",
    back_to_start: "Back to the start",
    title_not_found: "Not found",
    title_bad_request: "Check your request",
    title_unavailable: "Service unavailable",
    title_internal: "Something went wrong",
    message_internal: "The page could not be built. Please try again.",
    that_page: "that page",
    that_wood: "that wood",
    invalid_wood: "One of the selected woods is not valid.",

    fact_family: "Family",
    fact_density: "Density (apparent)",
    fact_shrinkage: "Volumetric shrinkage",
    fact_hardness: "Janka hardness",
    fact_bending: "Bending strength",
    fact_stiffness: "Stiffness (MOE)",
    fact_workability: "Ease of working",

    prop_hardness: "Hardness",
    prop_strength: "Bending strength",
    prop_stiffness: "Stiffness",
    prop_stability: "Dimensional stability",
    prop_lightness: "Light weight",
    prop_workability: "Ease of working",
    shrinkage_suffix: "shrinkage",
    ease_suffix: "ease",
};

pub static PT_BR: Strings = Strings {
    lang: Lang::PtBr,
    html_lang: "pt-BR",

    nav_ask: "Perguntar",
    nav_browse: "Ver madeiras",
    footer: "Dados: Laboratório de Produtos Florestais / Serviço Florestal Brasileiro. Os valores são indicativos; teste o seu próprio estoque antes de construir.",

    home_title: "Encontre a madeira certa",
    home_heading: "Qual madeira devo usar?",
    home_intro: "Descreva o que você quer construir, com suas próprias palavras. Classificamos para você as madeiras brasileiras do nosso banco de dados.",
    home_placeholder: "Quero fazer uma mesa de jantar para 12 lugares usando marcenaria japonesa",
    home_submit: "Buscar madeiras",

    answer_title: "Suas madeiras",
    answer_heading: "Madeiras para o seu projeto",
    understood: "Entendemos:",
    looked_at: "O que consideramos:",
    essential: "essencial",
    important: "importante",
    no_matches: "Nenhuma madeira do banco de dados tem os dados necessários para esta pergunta.",
    match_word: "compatibilidade",
    full_details: "Ver detalhes",
    compare_submit_range: "Comparar selecionadas (2 a 4)",
    in_short: "Em resumo",
    ask_another: "Fazer outra pergunta",
    explanation_cut_short: "A explicação foi interrompida.",

    browse_title: "Ver madeiras",
    browse_heading: "Madeiras brasileiras",
    col_name: "Nome",
    col_scientific_name: "Nome científico",
    col_family: "Família",
    col_hardness: "Dureza",
    col_density: "Densidade",
    compare_submit: "Comparar selecionadas",
    previous: "← Anterior",
    next: "Próxima →",

    also_called: "também chamada de",
    key_figures: "Principais características",
    typical_uses: "Usos típicos",
    working_it: "Trabalhabilidade",
    durability: "Durabilidade e preservação",
    general_characteristics: "Características gerais",
    the_tree: "A árvore",
    source: "Fonte",
    all_woods: "← Todas as madeiras",

    compare_title: "Comparar madeiras",
    compare_hint: "Maior dureza, resistência e rigidez suportam mais carga. Menor retração significa uma peça mais estável.",

    error_word: "Erro",
    back_to_start: "Voltar ao início",
    title_not_found: "Não encontrado",
    title_bad_request: "Verifique o pedido",
    title_unavailable: "Serviço indisponível",
    title_internal: "Algo deu errado",
    message_internal: "Não foi possível montar a página. Tente novamente.",
    that_page: "essa página",
    that_wood: "essa madeira",
    invalid_wood: "Uma das madeiras selecionadas não é válida.",

    fact_family: "Família",
    fact_density: "Densidade (aparente)",
    fact_shrinkage: "Retração volumétrica",
    fact_hardness: "Dureza Janka",
    fact_bending: "Resistência à flexão",
    fact_stiffness: "Rigidez (MOE)",
    fact_workability: "Facilidade de trabalho",

    prop_hardness: "Dureza",
    prop_strength: "Resistência à flexão",
    prop_stiffness: "Rigidez",
    prop_stability: "Estabilidade dimensional",
    prop_lightness: "Leveza",
    prop_workability: "Facilidade de trabalho",
    shrinkage_suffix: "de retração",
    ease_suffix: "de facilidade",
};

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn tags_are_matched_by_their_language_whatever_the_spelling() {
        for tag in ["pt", "pt-BR", "pt_br", " PT-pt ", "pt-BR-x-y"] {
            assert_eq!(Lang::from_tag(tag), Some(Lang::PtBr), "{tag}");
        }
        for tag in ["en", "en-GB", "EN_us"] {
            assert_eq!(Lang::from_tag(tag), Some(Lang::En), "{tag}");
        }
        for tag in ["", "fr", "*", "ptx", "p"] {
            assert_eq!(Lang::from_tag(tag), None, "{tag:?}");
        }
    }

    #[test]
    fn accept_language_prefers_the_highest_weight_then_the_first_listed() {
        let pick = Lang::from_accept_language;

        assert_eq!(pick("pt-BR,pt;q=0.9,en;q=0.8"), Some(Lang::PtBr));
        assert_eq!(pick("en;q=0.5, pt-BR;q=0.9"), Some(Lang::PtBr));
        assert_eq!(pick("fr, en;q=0.4"), Some(Lang::En));
        assert_eq!(
            pick("en, pt"),
            Some(Lang::En),
            "ties go to the first listed"
        );
        assert_eq!(pick("pt;q=0, en;q=0.1"), Some(Lang::En), "q=0 is refused");
        assert_eq!(
            pick("pt;q=high"),
            None,
            "an unreadable weight drops the entry"
        );
        assert_eq!(pick("fr, de;q=0.8, *;q=0.1"), None);
        assert_eq!(pick(""), None);
    }

    #[test]
    fn the_query_beats_the_cookie_which_beats_the_browser() {
        let browser = headers(&[(header::ACCEPT_LANGUAGE, "en")]);
        let both = headers(&[
            (header::ACCEPT_LANGUAGE, "en"),
            (header::COOKIE, "theme=dark; lang=pt-br"),
        ]);

        assert_eq!(
            Lang::detect(Some("x=1&lang=pt-br"), &both),
            (Lang::PtBr, Some(Lang::PtBr))
        );
        assert_eq!(
            Lang::detect(Some("lang=en"), &both),
            (Lang::En, Some(Lang::En))
        );
        assert_eq!(Lang::detect(None, &both), (Lang::PtBr, None));
        assert_eq!(Lang::detect(None, &browser), (Lang::En, None));
        assert_eq!(
            Lang::detect(Some("lang=xx"), &both),
            (Lang::PtBr, None),
            "an unknown language is ignored, not remembered"
        );
        assert_eq!(Lang::detect(None, &HeaderMap::new()), (Lang::En, None));
    }

    #[test]
    fn the_cookie_lasts_a_year_and_is_sent_only_to_this_site() {
        let cookie = Lang::PtBr.cookie();

        assert!(cookie.starts_with("lang=pt-br;"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=31536000"));
        assert!(cookie.contains("SameSite=Lax"));
    }

    #[test]
    fn every_language_has_its_own_code_and_strings() {
        for lang in Lang::ALL {
            assert_eq!(Lang::from_tag(lang.code()), Some(lang));
            assert_eq!(lang.strings().lang, lang);
        }
        assert_eq!(Lang::PtBr.strings().html_lang, "pt-BR");
    }

    #[test]
    fn decimals_follow_the_language() {
        assert_eq!(Lang::En.decimal("12.3 %".into()), "12.3 %");
        assert_eq!(Lang::PtBr.decimal("12.3 %".into()), "12,3 %");
    }

    #[test]
    fn sentences_with_numbers_are_translated() {
        let (en, pt) = (Lang::En.strings(), Lang::PtBr.strings());

        assert_eq!(en.better_than(85), "better than 85% of woods");
        assert_eq!(pt.better_than(85), "melhor que 85% das madeiras");
        assert!(en.showing(1, 24, 225).contains("1–24 of 225"));
        assert!(pt.showing(1, 24, 225).contains("1–24 de 225"));
        assert_eq!(
            pt.select_at_least(2),
            "Selecione pelo menos 2 madeiras para comparar."
        );
        assert_eq!(
            pt.compare_at_most(4),
            "Compare no máximo 4 madeiras por vez."
        );
        assert_eq!(
            pt.question_length(1000),
            "Escreva entre 1 e 1000 caracteres."
        );
        assert!(pt.unreachable("a API").contains("a API"));
        assert!(pt.rejected("a API", "vazio").ends_with("vazio"));
        assert!(pt.starting_up("a API").contains("iniciando"));
        assert!(pt.had_a_problem("a API").contains("problema"));
        assert_eq!(
            pt.could_not_find("essa madeira"),
            "Não encontramos essa madeira."
        );
        assert_eq!(pt.collected_in("Belém, PA"), "Coletada em Belém, PA.");
        assert_eq!(en.collected_in("Belém, PA"), "Collected in Belém, PA.");
        assert_eq!(pt.ranked_among(225), "Classificadas entre 225 madeiras.");
        assert_eq!(pt.compare_label("Ipê"), "Comparar Ipê");
    }
}
