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
    pub nav_history: &'static str,
    pub footer: &'static str,

    // Home.
    pub home_title: &'static str,
    pub home_heading: &'static str,
    pub home_intro: &'static str,
    pub home_examples: [&'static str; 3],
    pub home_tagline: &'static str,
    pub home_try: &'static str,
    pub home_explore: &'static str,
    pub home_browse_text: &'static str,
    pub home_compare_text: &'static str,
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
    pub thinking: &'static str,
    pub thinking_steps: [&'static str; 3],
    pub writing: &'static str,
    pub open_full_page: &'static str,
    pub close: &'static str,

    // The ask history, kept in the browser.
    pub history_heading: &'static str,
    pub history_new: &'static str,
    pub history_note: &'static str,
    pub history_empty: &'static str,
    pub history_clear: &'static str,
    pub history_clear_confirm: &'static str,
    pub history_remove: &'static str,
    pub history_saved_on: &'static str,
    pub history_missing: &'static str,

    // Browsing.
    pub browse_title: &'static str,
    pub browse_heading: &'static str,
    pub search_label: &'static str,
    pub search_placeholder: &'static str,
    pub search_button: &'static str,
    pub filters_heading: &'static str,
    pub filters_apply: &'static str,
    pub filters_clear: &'static str,
    pub facet_family: &'static str,
    pub facet_color: &'static str,
    pub facet_region: &'static str,
    pub facet_hardness: &'static str,
    pub facet_density: &'static str,
    pub facet_stability: &'static str,
    pub facet_figures: &'static str,
    pub hardness_range: &'static str,
    pub density_range: &'static str,
    pub range_min: &'static str,
    pub range_max: &'static str,
    pub only_with_photo: &'static str,
    /// In the order of `Hardness::ALL`, `Density::ALL`, `Stability::ALL` and `Sort::ALL`.
    pub hardness_classes: [&'static str; 4],
    pub density_classes: [&'static str; 3],
    pub stability_classes: [&'static str; 3],
    pub sort_label: &'static str,
    pub sort_labels: [&'static str; 8],
    pub view_label: &'static str,
    pub view_cards: &'static str,
    pub view_list: &'static str,
    pub no_results: &'static str,
    pub no_results_hint: &'static str,
    pub no_photo: &'static str,
    pub tray_compare: &'static str,
    pub tray_clear: &'static str,
    pub tray_hint: &'static str,
    pub tray_full: &'static str,
    pub col_shrinkage: &'static str,
    pub details_link: &'static str,
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
    pub compare_best: &'static str,
    pub compare_remove: &'static str,
    pub compare_add: &'static str,
    pub fact_color: &'static str,
    pub fact_region: &'static str,

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

    pub fn results_found(&self, found: usize, total: usize) -> String {
        match (self.lang, found == total) {
            (Lang::En, true) => format!("{found} woods"),
            (Lang::En, false) => format!("{found} of {total} woods"),
            (Lang::PtBr, true) => format!("{found} madeiras"),
            (Lang::PtBr, false) => format!("{found} de {total} madeiras"),
        }
    }

    pub fn page_of(&self, page: usize, pages: usize) -> String {
        match self.lang {
            Lang::En => format!("Page {page} of {pages}"),
            Lang::PtBr => format!("Página {page} de {pages}"),
        }
    }

    /// The name of a heartwood colour as the data gives it (Portuguese, lowercase).
    pub fn color_name(&self, color: &str) -> String {
        let english = match color {
            "marrom" => Some("Brown"),
            "amarela" | "amarelo" => Some("Yellow"),
            "branca" | "branco" => Some("White"),
            "cinza" => Some("Grey"),
            "rosa" => Some("Pink"),
            "vermelha" | "vermelho" => Some("Red"),
            "oliva" => Some("Olive"),
            "preta" | "preto" => Some("Black"),
            "roxa" | "roxo" => Some("Purple"),
            _ => None,
        };
        match (self.lang, english) {
            (Lang::En, Some(name)) => name.to_owned(),
            _ => {
                let mut letters = color.chars();
                letters
                    .next()
                    .map(|first| first.to_uppercase().chain(letters).collect())
                    .unwrap_or_default()
            }
        }
    }

    pub fn search_chip(&self, text: &str) -> String {
        format!("“{text}”")
    }

    /// "Janka ≥ 500", "Density ≤ 0.9" or "Janka 500–900", for a chip naming a range.
    pub fn range_chip(&self, label: &str, min: Option<f64>, max: Option<f64>) -> String {
        let number = |n: f64| self.lang.decimal(n.to_string());
        match (min, max) {
            (Some(min), Some(max)) => format!("{label}: {}–{}", number(min), number(max)),
            (Some(min), None) => format!("{label} ≥ {}", number(min)),
            (None, Some(max)) => format!("{label} ≤ {}", number(max)),
            (None, None) => label.to_owned(),
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
    nav_history: "History",
    footer: "Data: Laboratório de Produtos Florestais / Serviço Florestal Brasileiro. Figures are indicative; test your own stock before building.",

    home_title: "Wood Match",
    home_heading: "Wood Match",
    home_intro: "Tell us what you are building and our AI will shortlist the species in our database that suit it best, explaining the tradeoffs, the alternatives and the reasoning behind each pick.",
    home_examples: [
        "Which species holds up best on outdoor steps in a humid, coastal area?",
        "I need a stable wood for a wide kitchen countertop that won't warp.",
        "What is the easiest wood to shape by hand for a rocking chair?",
    ],
    home_tagline: "Ask for a recommendation, browse the whole database with advanced search, or compare species side by side.",
    home_try: "Try one of these",
    home_explore: "Or explore on your own",
    home_browse_text: "Look through every species in the database with its family, hardness and density.",
    home_compare_text: "Pick two to four species and see their figures side by side.",
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
    thinking: "Thinking…",
    thinking_steps: [
        "Reading your project",
        "Working out what matters: hardness, strength, stability…",
        "Ranking every wood in the database",
    ],
    writing: "Writing the explanation…",
    open_full_page: "Open full page ↗",
    close: "Close",

    history_heading: "History",
    history_new: "New question",
    history_note: "Kept in this browser only.",
    history_empty: "Your questions will appear here.",
    history_clear: "Clear history",
    history_clear_confirm: "Remove every saved question from this browser?",
    history_remove: "Remove",
    history_saved_on: "Saved",
    history_missing: "That answer is no longer saved in this browser.",

    browse_title: "Browse woods",
    browse_heading: "Brazilian woods",
    search_label: "Search the catalogue",
    search_placeholder: "Search by name, family, colour, place, use…",
    search_button: "Search",
    filters_heading: "Filters",
    filters_apply: "Apply filters",
    filters_clear: "Clear all",
    facet_family: "Family",
    facet_color: "Heartwood colour",
    facet_region: "Collection state",
    facet_hardness: "Hardness",
    facet_density: "Weight",
    facet_stability: "Stability",
    facet_figures: "Figures",
    hardness_range: "Janka hardness (kgf)",
    density_range: "Density (g/cm³)",
    range_min: "min",
    range_max: "max",
    only_with_photo: "Only woods with a photo",
    hardness_classes: ["Soft", "Medium hardness", "Hard", "Very hard"],
    density_classes: ["Light", "Medium weight", "Heavy"],
    stability_classes: [
        "Dimensionally stable",
        "Moderately stable",
        "Moves with humidity",
    ],
    sort_label: "Sort by",
    sort_labels: [
        "Best match",
        "Name A–Z",
        "Name Z–A",
        "Hardest first",
        "Softest first",
        "Heaviest first",
        "Lightest first",
        "Most stable first",
    ],
    view_label: "View",
    view_cards: "Cards",
    view_list: "List",
    no_results: "No wood matches this search.",
    no_results_hint: "Try fewer words or remove a filter.",
    no_photo: "No photo",
    tray_compare: "Compare",
    tray_clear: "Clear",
    tray_hint: "Pick at least two woods to compare",
    tray_full: "Four woods is the most you can compare",
    col_shrinkage: "Shrinkage",
    details_link: "Details",
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
    compare_best: "The best figure in each row is highlighted; the bars show each wood against the highest.",
    compare_remove: "Remove",
    compare_add: "Add another wood",
    fact_color: "Heartwood colour",
    fact_region: "Collected in",
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
    nav_history: "Histórico",
    footer: "Dados: Laboratório de Produtos Florestais / Serviço Florestal Brasileiro. Os valores são indicativos; teste o seu próprio estoque antes de construir.",

    home_title: "Madeira Ideal",
    home_heading: "Madeira Ideal",
    home_intro: "Conte o que você está construindo e nossa IA selecionará as espécies do nosso banco de dados que melhor atendem ao projeto, explicando os prós e contras, as alternativas e o raciocínio por trás de cada escolha.",
    home_examples: [
        "Qual espécie resiste melhor em degraus externos em uma região litorânea e úmida?",
        "Preciso de uma madeira estável para uma bancada de cozinha larga que não empene.",
        "Qual é a madeira mais fácil de moldar à mão para uma cadeira de balanço?",
    ],
    home_tagline: "Peça uma recomendação, explore todo o banco de dados com busca avançada ou compare espécies lado a lado.",
    home_try: "Experimente uma destas",
    home_explore: "Ou explore por conta própria",
    home_browse_text: "Percorra todas as espécies do banco de dados com sua família, dureza e densidade.",
    home_compare_text: "Escolha de duas a quatro espécies e veja seus dados lado a lado.",
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
    thinking: "Pensando…",
    thinking_steps: [
        "Lendo o seu projeto",
        "Avaliando o que importa: dureza, resistência, estabilidade…",
        "Classificando todas as madeiras do banco de dados",
    ],
    writing: "Escrevendo a explicação…",
    open_full_page: "Abrir página completa ↗",
    close: "Fechar",

    history_heading: "Histórico",
    history_new: "Nova pergunta",
    history_note: "Guardado apenas neste navegador.",
    history_empty: "Suas perguntas aparecerão aqui.",
    history_clear: "Limpar histórico",
    history_clear_confirm: "Remover todas as perguntas salvas deste navegador?",
    history_remove: "Remover",
    history_saved_on: "Salva em",
    history_missing: "Essa resposta não está mais salva neste navegador.",

    browse_title: "Ver madeiras",
    browse_heading: "Madeiras brasileiras",
    search_label: "Pesquisar no catálogo",
    search_placeholder: "Pesquise por nome, família, cor, local, uso…",
    search_button: "Pesquisar",
    filters_heading: "Filtros",
    filters_apply: "Aplicar filtros",
    filters_clear: "Limpar tudo",
    facet_family: "Família",
    facet_color: "Cor do cerne",
    facet_region: "Estado da coleta",
    facet_hardness: "Dureza",
    facet_density: "Peso",
    facet_stability: "Estabilidade",
    facet_figures: "Valores",
    hardness_range: "Dureza Janka (kgf)",
    density_range: "Densidade (g/cm³)",
    range_min: "mín.",
    range_max: "máx.",
    only_with_photo: "Somente madeiras com foto",
    hardness_classes: ["Macia", "Dureza média", "Dura", "Muito dura"],
    density_classes: ["Leve", "Peso médio", "Pesada"],
    stability_classes: [
        "Dimensionalmente estável",
        "Moderadamente estável",
        "Se move com a umidade",
    ],
    sort_label: "Ordenar por",
    sort_labels: [
        "Mais relevante",
        "Nome A–Z",
        "Nome Z–A",
        "Mais duras primeiro",
        "Mais macias primeiro",
        "Mais pesadas primeiro",
        "Mais leves primeiro",
        "Mais estáveis primeiro",
    ],
    view_label: "Exibição",
    view_cards: "Cartões",
    view_list: "Lista",
    no_results: "Nenhuma madeira corresponde a esta busca.",
    no_results_hint: "Tente menos palavras ou remova um filtro.",
    no_photo: "Sem foto",
    tray_compare: "Comparar",
    tray_clear: "Limpar",
    tray_hint: "Escolha pelo menos duas madeiras para comparar",
    tray_full: "Quatro madeiras é o máximo para comparar",
    col_shrinkage: "Retração",
    details_link: "Detalhes",
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
    compare_best: "O melhor valor de cada linha está em destaque; as barras mostram cada madeira em relação à maior.",
    compare_remove: "Remover",
    compare_add: "Adicionar outra madeira",
    fact_color: "Cor do cerne",
    fact_region: "Coletada em",
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
        assert_eq!(en.results_found(225, 225), "225 woods");
        assert_eq!(en.results_found(3, 225), "3 of 225 woods");
        assert_eq!(pt.results_found(225, 225), "225 madeiras");
        assert_eq!(pt.results_found(3, 225), "3 de 225 madeiras");
        assert_eq!(en.page_of(2, 9), "Page 2 of 9");
        assert_eq!(pt.page_of(2, 9), "Página 2 de 9");
        assert_eq!(en.search_chip("ipê"), "“ipê”");
        assert_eq!(
            en.range_chip("Janka", Some(500.0), Some(900.0)),
            "Janka: 500–900"
        );
        assert_eq!(pt.range_chip("Janka", Some(0.5), None), "Janka ≥ 0,5");
        assert_eq!(en.range_chip("Janka", None, Some(900.0)), "Janka ≤ 900");
        assert_eq!(en.range_chip("Janka", None, None), "Janka");
        assert_eq!(en.color_name("marrom"), "Brown");
        assert_eq!(pt.color_name("marrom"), "Marrom");
        assert_eq!(
            en.color_name("azulada"),
            "Azulada",
            "unknown colours are shown as given"
        );
        assert_eq!(en.color_name(""), "");
        for color in [
            "amarela", "amarelo", "branca", "branco", "cinza", "rosa", "vermelha", "vermelho",
            "oliva", "preta", "preto", "roxa", "roxo",
        ] {
            assert_ne!(
                en.color_name(color).to_lowercase(),
                color,
                "{color} is translated"
            );
        }
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
