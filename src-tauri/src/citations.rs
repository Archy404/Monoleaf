//! Rendering of citations and the reference list with hayagriva.
//!
//! The style and the locale come from the document header: `csl:` names one
//! of the bundled Citation Style Language styles, `lang:` picks the locale.
//! Custom style files are not read; a style name is all the header can give.

use crate::references::{self, header_string, Reference};
use hayagriva::archive::{locales, ArchivedStyle};
use hayagriva::citationberg::taxonomy::{Locator, Term};
use hayagriva::citationberg::{
    CitationFormat, IndependentStyle, Locale, LocaleCode, Style, StyleCategory, TermForm,
};
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

/// The locale from `lang:`; without one the style's own default locale,
/// as Pandoc does, and English (US) when the style names none either.
pub fn locale_named(lang: Option<&str>, style: &IndependentStyle) -> LocaleCode {
    match lang.map(str::trim).filter(|l| !l.is_empty()) {
        Some(lang) => LocaleCode(lang.to_string()),
        None => style
            .default_locale
            .clone()
            .unwrap_or_else(LocaleCode::en_us),
    }
}

/// One citation as the editor found it: `[@a; @b, p. 3]` has two items.
/// The field names are the frontend's; it sends what its parser found.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub items: Vec<CitedItem>,
    /// The in-text form `@key`: "Doe (2020)" rather than "(Doe 2020)".
    #[serde(default)]
    pub in_text: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitedItem {
    pub key: String,
    /// Text before the key, `see`; placed only for the first item of a group.
    #[serde(default)]
    pub prefix: String,
    /// Text after the key, `p. 3, emphasis added`: a locator at its start is
    /// read out, the rest is placed only for the last item of a group.
    #[serde(default)]
    pub suffix: String,
    /// `-@key`: the author is left out, "(2020)" in author-date styles.
    #[serde(default)]
    pub suppress_author: bool,
}

/// Every locator kind CSL knows; the locale holds the terms for each.
const LOCATOR_KINDS: [Locator; 29] = [
    Locator::Act,
    Locator::Appendix,
    Locator::ArticleLocator,
    Locator::Book,
    Locator::Canon,
    Locator::Chapter,
    Locator::Column,
    Locator::Elocation,
    Locator::Equation,
    Locator::Figure,
    Locator::Folio,
    Locator::Issue,
    Locator::Line,
    Locator::Note,
    Locator::Opus,
    Locator::Page,
    Locator::Paragraph,
    Locator::Part,
    Locator::Rule,
    Locator::Scene,
    Locator::Section,
    Locator::SubVerbo,
    Locator::Supplement,
    Locator::Table,
    Locator::Timestamp,
    Locator::Title,
    Locator::TitleLocator,
    Locator::Verse,
    Locator::Volume,
];

/// What a locale calls each locator kind, "p.", "pp.", "page", "pages" for
/// a page: the terms pandoc reads a locator by. The document's locale first,
/// then English, so "S. 3" and "p. 3" both work in a German document. The
/// longest term comes first, so "pp." is tried before "p.".
fn locator_terms(locales: &[Locale], code: &LocaleCode) -> Vec<(String, Locator)> {
    let mut terms = Vec::new();
    for lang in [code, &LocaleCode::en_us()] {
        let found = locales.iter().find(|l| l.lang.as_ref() == Some(lang));
        let Some(locale) = found else {
            continue;
        };
        for kind in LOCATOR_KINDS {
            for form in [TermForm::Long, TermForm::Short] {
                let Some(term) = locale.term(Term::Locator(kind), form) else {
                    continue;
                };
                for text in [term.single(), term.multiple()].into_iter().flatten() {
                    let text = text.trim_end_matches('.');
                    if !text.is_empty() {
                        terms.push((text.to_string(), kind));
                    }
                }
            }
        }
    }
    terms.sort_by_key(|(text, _)| std::cmp::Reverse(text.len()));
    terms
}

/// Pandoc's reading of what follows a key: a locator term with its number at
/// the start, "p. 3" or "chap. 2", or a bare number as a page; what follows
/// the number is the suffix. The term's dot is optional, the kind's own label
/// comes from the style. Nothing for a text that starts with neither.
fn split_locator<'a>(
    text: &'a str,
    terms: &[(String, Locator)],
) -> (Option<SpecificLocator<'a>>, &'a str) {
    let text = text.trim();
    let term = terms.iter().find(|(term, _)| {
        text.is_char_boundary(term.len())
            && text[..term.len()].eq_ignore_ascii_case(term)
            && text[term.len()..].starts_with(['.', ' '])
    });
    let (kind, rest) = match term {
        Some((term, kind)) => (*kind, text[term.len()..].trim_start_matches(['.', ' '])),
        None if text.starts_with(|c: char| c.is_ascii_digit()) => (Locator::Page, text),
        None => return (None, text),
    };
    // The number runs over digits, dashes, and a comma or space that leads
    // to another digit; "3-5, emphasis added" stops before the second comma.
    let bytes = rest.as_bytes();
    let leads_on = |n: usize| bytes.get(n).is_some_and(u8::is_ascii_digit);
    let mut end = 0;
    while end < bytes.len() {
        let c = bytes[end];
        let joins = (c == b',' || c == b' ') && (leads_on(end + 1) || leads_on(end + 2));
        if c.is_ascii_digit() || c == b'-' || joins {
            end += 1;
        } else if rest[end..].starts_with('\u{2013}') {
            end += '\u{2013}'.len_utf8();
        } else {
            break;
        }
    }
    let number = rest[..end].trim_end_matches([',', ' ']);
    let suffix = rest[end..].trim_start_matches([',', ' ']);
    (
        Some(SpecificLocator(kind, LocatorPayload::Str(number))),
        suffix,
    )
}

fn is_numeric(style: &IndependentStyle) -> bool {
    let numeric = |category: &StyleCategory| {
        matches!(
            category,
            StyleCategory::CitationFormat {
                format: CitationFormat::Numeric
            }
        )
    };
    style.info.category.iter().any(numeric)
}

/// The bracket a rendered citation opens with: the style's own, else a plain
/// one, as numeric styles print theirs from inside the layout.
fn opening<'h>(html: &'h str, style: &IndependentStyle) -> Option<&'h str> {
    let own = style.citation.layout.prefix.as_deref().unwrap_or("");
    [own, "(", "["]
        .into_iter()
        .find(|b| !b.is_empty() && html.starts_with(b))
        .map(|b| &html[..b.len()])
}

fn closing<'h>(html: &'h str, style: &IndependentStyle) -> Option<&'h str> {
    let own = style.citation.layout.suffix.as_deref().unwrap_or("");
    [own, ")", "]"]
        .into_iter()
        .find(|b| !b.is_empty() && html.ends_with(b))
        .map(|b| &html[html.len() - b.len()..])
}

/// Pandoc's extras hayagriva has no item for. A suppressed author cuts the
/// prose form "Doe (2020, p. 3)" at its bracket; the first item's prefix goes
/// in after the opening bracket, the last item's suffix before the closing one.
fn decorate(
    mut html: String,
    citation: &Citation,
    suffixes: &[&str],
    style: &IndependentStyle,
) -> String {
    let (Some(first), Some(suffix)) = (citation.items.first(), suffixes.last()) else {
        return html;
    };
    if suppresses_author(citation, style) {
        let bracket = style.citation.layout.prefix.as_deref().unwrap_or("(");
        if let Some(at) = html.find(bracket) {
            html.replace_range(..at, "");
        }
    }
    let prefix = first.prefix.trim();
    if !prefix.is_empty() {
        let at = opening(&html, style).map_or(0, str::len);
        html.insert_str(at, &format!("{prefix} "));
    }
    let suffix = suffix.trim();
    if !suffix.is_empty() {
        let at = html.len() - closing(&html, style).map_or(0, str::len);
        html.insert_str(at, &format!(", {suffix}"));
    }
    html
}

/// Only a citation of one work drops its author; numeric styles never do.
fn suppresses_author(citation: &Citation, style: &IndependentStyle) -> bool {
    citation.items.len() == 1 && citation.items[0].suppress_author && !is_numeric(style)
}

/// What the export prints: the citations in order and the reference list.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedCitations {
    /// One per citation given; `None` where it stays as written.
    pub citations: Vec<Option<String>>,
    pub bibliography: Vec<BibliographyEntry>,
    pub hanging_indent: bool,
    /// The heading over the list, from `reference-section-title:`; none
    /// without it, as pandoc prints the list.
    pub title: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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

/// Renders the citations in document order and the reference list, in one
/// hayagriva run over every reference.
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
        by_key.entry(reference.key()).or_insert(&reference.item);
    }
    let locales = locales();
    let terms = locator_terms(&locales, locale);
    // The suffix of each item once its locator is read out, for decorate().
    let suffixes: Vec<Vec<&str>> = citations
        .iter()
        .map(|c| {
            c.items
                .iter()
                .map(|i| split_locator(&i.suffix, &terms).1)
                .collect()
        })
        .collect();
    let mut cited = Vec::new();
    for (index, citation) in citations.iter().enumerate() {
        let mut items = Vec::new();
        let mut known = true;
        for item in &citation.items {
            match by_key.get(&item.key) {
                Some(reference) => items.push((*reference, item)),
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
        let inner = citation.items.iter().enumerate().any(|(i, item)| {
            (i > 0 && !item.prefix.trim().is_empty())
                || (i + 1 < citation.items.len() && !suffixes[index][i].trim().is_empty())
        });
        if inner {
            out.warnings.push(format!(
                "citation {}: only the first prefix and the last suffix of a group are printed",
                index + 1
            ));
        }
        if citation.items.len() > 1 && citation.items.iter().any(|item| item.suppress_author) {
            out.warnings.push(format!(
                "citation {}: -@ is ignored inside a group",
                index + 1
            ));
        }
        if known {
            cited.push((index, items));
        }
    }
    if cited.is_empty() {
        return out;
    }
    let mut driver = BibliographyDriver::new();
    for (index, items) in &cited {
        let citation = &citations[*index];
        let items = items
            .iter()
            .map(|(reference, cited)| {
                let locator = split_locator(&cited.suffix, &terms).0;
                let item = CitationItem::with_locator(*reference, locator);
                match citation.in_text || suppresses_author(citation, style) {
                    true => item.kind(CitePurpose::Prose),
                    false => item,
                }
            })
            .collect();
        let request = CitationRequest::new(items, style, Some(locale.clone()), &locales, None);
        driver.citation(request);
    }
    let request = BibliographyRequest::new(style, Some(locale.clone()), &locales);
    let rendered = driver.finish(request);
    for ((index, _), citation) in cited.iter().zip(rendered.citations) {
        let text = html(&citation.citation);
        let text = decorate(text, &citations[*index], &suffixes[*index], style);
        out.citations[*index] = Some(text);
    }
    if let Some(list) = rendered.bibliography {
        out.hanging_indent = list.hanging_indent;
        out.bibliography = list
            .items
            .into_iter()
            .map(|item| BibliographyEntry {
                key: item.key,
                label: item.first_field.as_ref().map(html_of),
                html: html(&item.content),
            })
            .collect();
    }
    out
}

/// The export's one call: reads the references the header names, takes the
/// style and the locale from the header, and renders. The loader's warnings
/// come first in the result, each named after its source.
pub fn render_document(
    document_path: Option<&str>,
    front_matter: &str,
    allow_network: bool,
    citations: &[Citation],
) -> Result<RenderedCitations, String> {
    let style = style_named(header_string(front_matter, "csl").as_deref())?;
    let locale = locale_named(header_string(front_matter, "lang").as_deref(), &style);
    let loaded = references::load_references(document_path, front_matter, allow_network);
    let mut out = render(&loaded.references, &style, &locale, citations);
    out.title = header_string(front_matter, "reference-section-title");
    out.warnings.splice(0..0, loaded.warnings);
    Ok(out)
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
    fn locale_comes_from_the_header_then_the_style_then_en_us() {
        let apa = style_named(Some("apa")).unwrap();
        let thieme = style_named(Some("thieme")).unwrap();
        assert_eq!(locale_named(None, &apa), LocaleCode::en_us());
        assert_eq!(locale_named(None, &thieme).0, "de-DE");
        assert_eq!(locale_named(Some("en-GB"), &thieme).0, "en-GB");
    }

    #[test]
    fn the_issue_example_renders_in_the_default_and_in_apa() {
        let references = read_csl_yaml(ISSUE_EXAMPLE).unwrap().references;
        let cited = [cite(&["ERA5LandHourlyData2019"])];
        let style = style_named(None).unwrap();
        let locale = locale_named(None, &style);
        let out = render(&references, &style, &locale, &cited);
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
    fn bib_and_csl_references_share_one_numbering() {
        let mut references = read_biblatex(BIB, true).references;
        references.extend(read_csl_yaml(ISSUE_EXAMPLE).unwrap().references);
        let mut page = cite(&["doe2020"]);
        page.items[0].suffix = "3".into();
        let cited = [
            cite(&["ERA5LandHourlyData2019"]),
            page,
            cite(&["doe2020", "ERA5LandHourlyData2019"]),
            cite(&["nobody"]),
        ];
        let style = style_named(Some("ieee")).unwrap();
        let out = render(&references, &style, &locale_named(None, &style), &cited);
        assert_eq!(out.citations[0].as_deref(), Some("[1]"));
        assert_eq!(out.citations[1].as_deref(), Some("[2, p. 3]"));
        assert_eq!(out.citations[2].as_deref(), Some("[1], [2]"));
        assert_eq!(out.citations[3], None);
        let keys: Vec<_> = out.bibliography.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["ERA5LandHourlyData2019", "doe2020"]);
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("nobody"));
    }

    #[test]
    fn prefix_suffix_locator_and_suppressed_author_print_as_pandoc_does() {
        let references = read_biblatex(BIB, true).references;
        let mut seen = cite(&["doe2020"]);
        seen.items[0].prefix = "see".into();
        seen.items[0].suffix = "chap. 2, emphasis added".into();
        let mut year = cite(&["doe2020"]);
        year.items[0].suppress_author = true;
        year.items[0].suffix = "p. 3".into();
        let mut page = cite(&["doe2020"]);
        page.items[0].suffix = "12".into();
        let mut group = cite(&["doe2020", "doe2020"]);
        group.items[1].prefix = "also".into();
        group.items[1].suppress_author = true;
        let cited = [seen, year, page, group];
        let locale = LocaleCode::en_us();
        let apa = render(
            &references,
            &style_named(Some("apa")).unwrap(),
            &locale,
            &cited,
        );
        let texts: Vec<_> = apa
            .citations
            .iter()
            .map(|c| c.as_deref().unwrap())
            .collect();
        assert_eq!(
            texts[..3],
            [
                "(see Doe & Roe, 2020, Chapter 2, emphasis added)",
                "(2020, p. 3)",
                "(Doe & Roe, 2020, p. 12)"
            ]
        );
        assert_eq!(apa.warnings.len(), 2, "{:?}", apa.warnings);
        let ieee = render(
            &references,
            &style_named(Some("ieee")).unwrap(),
            &locale,
            &cited,
        );
        assert_eq!(
            ieee.citations[0].as_deref(),
            Some("[see 1, Ch. 2, emphasis added]")
        );
        assert_eq!(ieee.citations[1].as_deref(), Some("[1, p. 3]"));
    }

    #[test]
    fn locator_terms_come_from_the_locale_and_english() {
        let references = read_biblatex(BIB, true).references;
        let texts = [
            "pp. 3-5, emphasis added",
            "Kap 2",
            "S. 7",
            "page 9, see",
            "nothing numeric",
        ];
        let cited: Vec<Citation> = texts
            .iter()
            .map(|text| {
                let mut c = cite(&["doe2020"]);
                c.items[0].suffix = text.to_string();
                c
            })
            .collect();
        let style = style_named(Some("apa")).unwrap();
        let out = render(
            &references,
            &style,
            &locale_named(Some("de-DE"), &style),
            &cited,
        );
        let texts: Vec<_> = out
            .citations
            .iter()
            .map(|c| c.as_deref().unwrap())
            .collect();
        assert_eq!(
            texts,
            [
                "(Doe & Roe, 2020, S. 3\u{2013}5, emphasis added)",
                "(Doe & Roe, 2020, Kapitel 2)",
                "(Doe & Roe, 2020, S. 7)",
                "(Doe & Roe, 2020, S. 9, see)",
                "(Doe & Roe, 2020, nothing numeric)"
            ]
        );
    }

    #[test]
    fn a_document_renders_from_its_header_alone() {
        let header = format!(
            "csl: apa\nlang: en-US\nreference-section-title: Sources\n{ISSUE_EXAMPLE}bibliography: gone.bib\n"
        );
        let cited = [cite(&["ERA5LandHourlyData2019"])];
        let document = std::env::temp_dir().join("paper.md");
        let out = render_document(document.to_str(), &header, false, &cited).unwrap();
        assert_eq!(
            out.citations[0].as_deref(),
            Some("(Copernicus Climate Change Service, 2019)")
        );
        assert_eq!(out.warnings, ["gone.bib: not found"]);
        assert_eq!(out.title.as_deref(), Some("Sources"));
        let json = serde_json::to_value(&out).unwrap();
        assert!(json.get("hangingIndent").is_some());
        let sent: Citation =
            serde_json::from_str(r#"{"items":[{"key":"a"}],"inText":true}"#).unwrap();
        assert!(sent.in_text && sent.items[0].key == "a");
        let err = render_document(None, "csl: nope\n", false, &cited).unwrap_err();
        assert!(err.starts_with("unknown citation style"));
    }

    /// The header of a fixture document, between its `---` lines.
    fn front_matter(text: &str) -> &str {
        let body = text.strip_prefix("---\n").unwrap_or("");
        body.split("\n---\n").next().unwrap_or("")
    }

    /// The citation chain's Rust link (see src/citations.fixtures.test.ts):
    /// `name.citations.json`, what the export sends for `name.md`, renders
    /// with the document's header to `name.rendered.json`, which the export
    /// snapshot feeds back in. MONOLEAF_UPDATE_FIXTURES=1 rewrites the
    /// rendered files after an intended change.
    #[test]
    fn fixture_documents_render_as_committed() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/__fixtures__/citations");
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let base = path.with_extension("").display().to_string();
            let text = std::fs::read_to_string(&path).unwrap();
            let sent = std::fs::read_to_string(format!("{base}.citations.json")).unwrap();
            let cited: Vec<Citation> = serde_json::from_str(&sent).unwrap();
            let out = render_document(path.to_str(), front_matter(&text), false, &cited).unwrap();
            let json = serde_json::to_string_pretty(&out).unwrap() + "\n";
            let rendered = format!("{base}.rendered.json");
            if std::env::var_os("MONOLEAF_UPDATE_FIXTURES").is_some() {
                std::fs::write(&rendered, &json).unwrap();
            } else {
                let committed = std::fs::read_to_string(&rendered).unwrap_or_default();
                assert_eq!(committed, json, "{rendered} differs from the rendering; MONOLEAF_UPDATE_FIXTURES=1 cargo test rewrites it");
            }
            seen += 1;
        }
        assert!(seen > 0, "no fixture documents in {}", dir.display());
    }
}
