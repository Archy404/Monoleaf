//! Rendering of citations and the reference list with hayagriva.
//!
//! The style and the locale come from the document header: `csl:` names one
//! of the bundled Citation Style Language styles, `lang:` picks the locale.
//! Custom style files are not read; a style name is all the header can give.

use crate::references::Reference;
use hayagriva::archive::{locales, ArchivedStyle};
use hayagriva::citationberg::taxonomy::Locator;
use hayagriva::citationberg::{IndependentStyle, LocaleCode, Style};
use hayagriva::{
    BibliographyDriver, BibliographyRequest, BufWriteFormat, CitationItem, CitationRequest,
    CitePurpose, ElemChild, ElemChildren, LocatorPayload, SpecificLocator,
};
use std::collections::HashMap;

/// Pandoc's default when the header names no style.
const DEFAULT_STYLE: &str = "chicago-author-date";
/// Styles named in the error for an unknown style, so the user sees the form.
const EXAMPLE_STYLES: &str = "apa, ieee, chicago-author-date, vancouver, nature";

/// The bundled style of the given name, or the default when the header has none.
pub fn style_named(name: Option<&str>) -> Result<IndependentStyle, String> {
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_STYLE);
    let archived = ArchivedStyle::by_name(name)
        .ok_or_else(|| format!("unknown citation style \"{name}\"; try one of {EXAMPLE_STYLES}"))?;
    match archived.get() {
        Style::Independent(style) => Ok(style),
        Style::Dependent(_) => Err(format!(
            "citation style \"{name}\" depends on another style"
        )),
    }
}

/// The locale from `lang:`, English (US) when the header has none.
pub fn locale_named(lang: Option<&str>) -> LocaleCode {
    match lang.map(str::trim).filter(|l| !l.is_empty()) {
        Some(lang) => LocaleCode(lang.to_string()),
        None => LocaleCode::en_us(),
    }
}

/// One citation as the editor found it: `[@a; @b, p. 3]` has two items.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Citation {
    pub items: Vec<CitedItem>,
    /// The in-text form `@key`: "Doe (2020)" rather than "(Doe 2020)".
    #[serde(default)]
    pub in_text: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct CitedItem {
    pub key: String,
    /// What follows the key, `p. 3`; read as a page until labels are parsed.
    #[serde(default)]
    pub locator: Option<String>,
}

/// What the export prints: the citations in order and the reference list.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct RenderedCitations {
    /// One per citation given; `None` where it stays as written.
    pub citations: Vec<Option<String>>,
    pub bibliography: Vec<BibliographyEntry>,
    pub hanging_indent: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct BibliographyEntry {
    pub key: String,
    /// What numeric styles print in front of the entry, `[1]`.
    pub label: Option<String>,
    pub html: String,
}

fn html(children: &ElemChildren) -> String {
    let mut out = String::new();
    let _ = children.write_buf(&mut out, BufWriteFormat::Html);
    out
}

fn html_of(child: &ElemChild) -> String {
    let mut out = String::new();
    let _ = child.write_buf(&mut out, BufWriteFormat::Html);
    out
}

fn key_of(reference: &Reference) -> String {
    match reference {
        Reference::Bib(entry) => entry.key().to_string(),
        Reference::Csl(item) => item.id().unwrap_or_default().into_owned(),
    }
}

/// Renders the citations in document order and the reference list.
///
/// hayagriva drives one entry type per run and keeps the trait that could
/// join the two private, so the `.bib` references go through one run and the
/// CSL references through a second one that continues the numbering. That
/// loses numbering by first appearance, one sorted list and disambiguation
/// across the two sets; a citation grouping both kinds stays as written.
pub fn render(
    references: &[Reference],
    style: &IndependentStyle,
    locale: &LocaleCode,
    citations: &[Citation],
) -> RenderedCitations {
    let mut out = RenderedCitations {
        citations: vec![None; citations.len()],
        ..Default::default()
    };
    let mut by_key = HashMap::new();
    for reference in references {
        by_key.entry(key_of(reference)).or_insert(reference);
    }
    let mut bib = Vec::new();
    let mut csl = Vec::new();
    for (index, citation) in citations.iter().enumerate() {
        let mut entries = Vec::new();
        let mut items = Vec::new();
        let mut known = true;
        for item in &citation.items {
            match by_key.get(&item.key) {
                Some(Reference::Bib(entry)) => {
                    entries.push((entry.as_ref(), item.locator.as_deref()))
                }
                Some(Reference::Csl(csl_item)) => items.push((csl_item, item.locator.as_deref())),
                None => {
                    out.warnings.push(format!(
                        "citation {}: no reference has the key {}",
                        index + 1,
                        item.key
                    ));
                    known = false;
                }
            }
        }
        match (known, entries.is_empty(), items.is_empty()) {
            (true, false, true) => bib.push((index, entries)),
            (true, true, false) => csl.push((index, items)),
            (true, false, false) => out.warnings.push(format!(
                "citation {}: a .bib and a CSL reference in one citation cannot be rendered together",
                index + 1
            )),
            _ => {}
        }
    }
    let locales = locales();
    // The same run for both entry types; a macro, since the type that would
    // make this a generic function is hayagriva's private trait.
    macro_rules! run {
        ($cited:expr, $offset:expr) => {{
            let mut driver = BibliographyDriver::new().with_citation_number_offset($offset);
            for (index, entries) in &$cited {
                let items = entries
                    .iter()
                    .map(|(entry, locator)| {
                        let locator = locator
                            .map(|page| SpecificLocator(Locator::Page, LocatorPayload::Str(page)));
                        let item = CitationItem::with_locator(*entry, locator);
                        match citations[*index].in_text {
                            true => item.kind(CitePurpose::Prose),
                            false => item,
                        }
                    })
                    .collect();
                let request =
                    CitationRequest::new(items, style, Some(locale.clone()), &locales, None);
                driver.citation(request);
            }
            let request = BibliographyRequest::new(style, Some(locale.clone()), &locales);
            let rendered = driver.finish(request);
            for ((index, _), citation) in $cited.iter().zip(rendered.citations) {
                out.citations[*index] = Some(html(&citation.citation));
            }
            if let Some(list) = rendered.bibliography {
                out.hanging_indent = list.hanging_indent;
                out.bibliography
                    .extend(list.items.into_iter().map(|item| BibliographyEntry {
                        key: item.key,
                        label: item.first_field.as_ref().map(html_of),
                        html: html(&item.content),
                    }));
            }
        }};
    }
    if !bib.is_empty() {
        run!(bib, 0);
    }
    if !csl.is_empty() {
        if !bib.is_empty() {
            out.warnings.push(
                "the document cites .bib and CSL references: each format is numbered and listed \
                 on its own, and two works that cite alike are not told apart"
                    .into(),
            );
        }
        run!(csl, out.bibliography.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::references::{read_biblatex, read_csl_yaml};

    const BIB: &str = "@article{doe2020, author={Doe, Jane and Roe, Richard}, title={A Study}, \
                       journal={J. Stud.}, year={2020}, volume={3}, pages={1--9}}";
    const ISSUE_EXAMPLE: &str = "references:\n- id: ERA5LandHourlyData2019\n  type: dataset\n  \
                                 title: ERA5-Land hourly data from 1950 to present\n  author:\n  \
                                 - literal: Copernicus Climate Change Service\n  issued: 2019\n";

    fn cite(keys: &[&str]) -> Citation {
        let items = keys.iter().map(|key| CitedItem {
            key: key.to_string(),
            ..Default::default()
        });
        Citation {
            items: items.collect(),
            in_text: false,
        }
    }

    #[test]
    fn style_defaults_to_chicago_author_date() {
        let style = style_named(None).unwrap();
        assert!(style.info.id.ends_with("/styles/chicago-author-date"));
        assert_eq!(style_named(Some("  ")).unwrap().info.id, style.info.id);
    }

    #[test]
    fn style_is_found_by_name() {
        let style = style_named(Some(" apa ")).unwrap();
        assert!(style.info.id.ends_with("/styles/apa"), "{}", style.info.id);
    }

    #[test]
    fn unknown_style_names_examples() {
        let err = style_named(Some("harvard")).unwrap_err();
        assert!(err.contains("\"harvard\"") && err.contains("apa"));
    }

    #[test]
    fn locale_defaults_to_en_us() {
        assert_eq!(locale_named(None), LocaleCode::en_us());
        assert_eq!(locale_named(Some("de-DE")).0, "de-DE");
    }

    #[test]
    fn the_issue_example_renders_in_the_default_and_in_apa() {
        let references = read_csl_yaml(ISSUE_EXAMPLE).unwrap().references;
        let cited = [cite(&["ERA5LandHourlyData2019"])];
        let locale = locale_named(None);
        let out = render(&references, &style_named(None).unwrap(), &locale, &cited);
        assert_eq!(
            out.citations,
            [Some("(Copernicus Climate Change Service 2019)".into())]
        );
        assert!(out.bibliography[0]
            .html
            .starts_with("Copernicus Climate Change Service. 2019."));
        assert!(out.hanging_indent && out.warnings.is_empty());
        let apa = render(
            &references,
            &style_named(Some("apa")).unwrap(),
            &locale,
            &cited,
        );
        assert_eq!(
            apa.citations,
            [Some("(Copernicus Climate Change Service, 2019)".into())]
        );
    }

    #[test]
    fn bib_and_csl_references_are_numbered_one_after_the_other() {
        let mut references = read_biblatex(BIB).references;
        references.extend(read_csl_yaml(ISSUE_EXAMPLE).unwrap().references);
        let mut page = cite(&["doe2020"]);
        page.items[0].locator = Some("3".into());
        let cited = [
            cite(&["ERA5LandHourlyData2019"]),
            page,
            cite(&["doe2020", "ERA5LandHourlyData2019"]),
            cite(&["nobody"]),
        ];
        let style = style_named(Some("ieee")).unwrap();
        let out = render(&references, &style, &locale_named(None), &cited);
        assert_eq!(out.citations[0].as_deref(), Some("[2]"));
        assert_eq!(out.citations[1].as_deref(), Some("[1, p. 3]"));
        assert_eq!((&out.citations[2], &out.citations[3]), (&None, &None));
        let labels: Vec<_> = out
            .bibliography
            .iter()
            .map(|e| e.label.as_deref())
            .collect();
        assert_eq!(labels, [Some("[1]"), Some("[2]")]);
        assert_eq!(out.warnings.len(), 3, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("citation 3:") && out.warnings[1].contains("nobody"));
    }
}
