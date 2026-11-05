use anyhow::{Result, anyhow, bail};
use scraper::{ElementRef, Selector};

pub(crate) fn query(value: &str) -> Result<Selector> {
    Selector::parse(value).map_err(|_| anyhow!("invalid CSS selector: {value}"))
}

pub(crate) fn text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn table_rows_by_header(
    container: ElementRef<'_>,
    header: &str,
) -> Result<Vec<Vec<String>>> {
    let th_selector = query("th")?;
    let tr_selector = query("tr")?;
    let td_selector = query("td")?;
    let table_selector = query("table")?;

    for table in container.select(&table_selector) {
        let mut column_start = None;
        let mut column_count = 1;
        let mut current_column = 0;

        for th in table.select(&th_selector) {
            let th_text = th
                .text()
                .collect::<String>()
                .trim()
                .to_lowercase()
                .to_string();

            let colspan: usize = th
                .value()
                .attr("colspan")
                .unwrap_or("1")
                .parse()
                .unwrap_or(1);

            if th_text == header.to_lowercase() {
                column_start = Some(current_column);
                column_count = colspan;
                break;
            }

            current_column += colspan;
        }

        let column_start = match column_start {
            Some(value) => value,
            None => continue,
        };

        let mut result = Vec::new();

        for tr in table.select(&tr_selector) {
            let cells: Vec<String> = tr
                .select(&td_selector)
                .map(|td| td.text().collect::<String>().trim().to_string())
                .collect();

            let values = cells
                .iter()
                .skip(column_start)
                .take(column_count)
                .cloned()
                .collect::<Vec<String>>();

            if values.is_empty() {
                continue;
            }

            result.push(values);
        }

        return Ok(result);
    }

    bail!("table column with header {header} not found")
}

pub(crate) fn table_rows_by_heading(
    container: ElementRef<'_>,
    heading: &str,
) -> Result<Vec<Vec<String>>> {
    let row_selector = query("tr")?;
    let cell_selector = query("th, td")?;
    let h3_selector = query("h3")?;
    let table_selector = query("table")?;

    for h3 in container.select(&h3_selector) {
        let title = h3
            .text()
            .collect::<String>()
            .trim()
            .to_lowercase()
            .to_string();

        if title == heading.to_lowercase() {
            let table = h3
                .parent()
                .and_then(ElementRef::wrap)
                .and_then(|parent| parent.select(&table_selector).next())
                .ok_or_else(|| anyhow!("table {heading} has no table next to its heading"))?;

            return Ok(table_rows(table, &row_selector, &cell_selector));
        }
    }

    bail!("table {} not found", heading)
}

pub(crate) fn table_rows(
    table: ElementRef<'_>,
    row_selector: &Selector,
    cell_selector: &Selector,
) -> Vec<Vec<String>> {
    table
        .select(row_selector)
        .map(|row| table_cell_texts(row, cell_selector))
        .filter(|row| !row.is_empty())
        .collect()
}

fn table_cell_texts(element: ElementRef<'_>, selector: &Selector) -> Vec<String> {
    element
        .select(selector)
        .map(text)
        // .filter(|value| !value.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use scraper::Html;

    use super::*;

    fn root(document: &Html) -> ElementRef<'_> {
        document.select(&query("body").unwrap()).next().unwrap()
    }

    #[test]
    fn query_rejects_invalid_selectors() {
        assert!(query("div > a").is_ok());
        assert_eq!(
            query("[[").unwrap_err().to_string(),
            "invalid CSS selector: [["
        );
    }

    #[test]
    fn text_flattens_nested_markup_and_whitespace() {
        let document = Html::parse_fragment("<p>  Olá <b>mundo</b>\n  !  </p>");
        let p = document.select(&query("p").unwrap()).next().unwrap();

        assert_eq!(text(p), "Olá mundo !");
    }

    #[test]
    fn table_rows_by_header_returns_the_columns_under_the_header() {
        let document = Html::parse_document(
            "<body><table><tr><th>A</th><th colspan=2>B</th></tr>\
             <tr><td>a1</td><td>b1</td><td>b2</td></tr>\
             <tr><td>a2</td><td>b3</td></tr></table></body>",
        );

        let rows = table_rows_by_header(root(&document), "b").unwrap();

        assert_eq!(rows, vec![vec!["b1", "b2"], vec!["b3"]]);
    }

    #[test]
    fn table_rows_by_header_skips_tables_without_the_header_and_short_rows() {
        let document = Html::parse_document(
            "<body><table><tr><th>X</th></tr><tr><td>x</td></tr></table>\
             <table><tr><th>A</th><th colspan=oops>B</th></tr><tr><td>only-a</td></tr><tr><td>a</td><td>b</td></tr></table></body>",
        );

        let rows = table_rows_by_header(root(&document), "B").unwrap();

        assert_eq!(rows, vec![vec!["b"]]);
    }

    #[test]
    fn table_rows_by_header_fails_when_no_table_has_the_header() {
        let document = Html::parse_document("<body><table><tr><th>A</th></tr></table></body>");

        let error = table_rows_by_header(root(&document), "Z").unwrap_err();

        assert_eq!(error.to_string(), "table column with header Z not found");
    }

    #[test]
    fn table_rows_by_heading_reads_the_table_next_to_the_heading() {
        let document = Html::parse_document(
            "<body><div><h3> Título </h3><table><tr><th>h1</th><th>h2</th></tr>\
             <tr><td>a</td><td> b </td></tr><tr></tr></table></div></body>",
        );

        let rows = table_rows_by_heading(root(&document), "TÍTULO").unwrap();

        assert_eq!(rows, vec![vec!["h1", "h2"], vec!["a", "b"]]);
    }

    #[test]
    fn table_rows_by_heading_fails_when_the_heading_has_no_table() {
        let document = Html::parse_document("<body><div><h3>Título</h3></div></body>");

        let error = table_rows_by_heading(root(&document), "Título").unwrap_err();

        assert_eq!(
            error.to_string(),
            "table Título has no table next to its heading"
        );
    }

    #[test]
    fn table_rows_by_heading_fails_when_the_heading_is_missing() {
        let document = Html::parse_document("<body><div><h3>Outro</h3></div></body>");

        let error = table_rows_by_heading(root(&document), "Título").unwrap_err();

        assert_eq!(error.to_string(), "table Título not found");
    }
}
