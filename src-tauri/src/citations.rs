//! Rendering of citations and the reference list with hayagriva.
//!
//! The style and the locale come from the document header: `csl:` names one
//! of the bundled Citation Style Language styles, `lang:` picks the locale.
//! Custom style files are not read; a style name is all the header can give.

use crate::references::{self, header_string, Reference};
use hayagriva::archive::{locales, ArchivedStyle};
use hayagriva::citationberg::taxonomy::{Locator, Term};
use hayagriva::citationberg::{
    CitationFormat, IndependentStyle, Locale, LocaleCode, Style, StyleCategory, StyleClass,
    TermForm,
};
use hayagriva::{
    BibliographyDriver, BibliographyRequest, BufWriteFormat, CitationItem, CitationRequest,
    CitePurpose, Elem, ElemChild, ElemChildren, ElemMeta, Formatted, Formatting, LocatorPayload,
    SpecificLocator,
};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

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

/// What the references dialog offers: every bundled style by its shortest
/// name, "apa" rather than "american-psychological-association", and every
/// bundled locale by its code, each list sorted.
#[derive(Debug, serde::Serialize)]
pub struct Catalogue {
    pub styles: Vec<String>,
    pub locales: Vec<String>,
}

pub fn catalogue() -> Catalogue {
    let mut styles: Vec<String> = ArchivedStyle::all()
        .iter()
        .filter(|s| matches!(s.get(), Style::Independent(_)))
        .filter_map(|s| s.names().iter().min_by_key(|n| n.len()))
        .map(|n| n.to_string())
        .collect();
    styles.sort();
    let mut locales: Vec<String> = bundled_locales()
        .iter()
        .filter_map(|l| l.lang.as_ref().map(|code| code.0.clone()))
        .collect();
    locales.sort();
    locales.dedup();
    Catalogue { styles, locales }
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
    /// Inside a footnote, not at its start: a note style prints it in
    /// parentheses, as the footnote already is the note.
    #[serde(default)]
    pub in_note: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitedItem {
    pub key: String,
    /// Text before the key, `see`.
    #[serde(default)]
    pub prefix: String,
    /// Text after the key, `p. 3, emphasis added`: a locator at its start is
    /// read out, the rest follows the item.
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

/// hayagriva's bundled locales, parsed once: parsing them costs a tenth of a
/// second, and every export and live-view render would pay it again.
fn bundled_locales() -> &'static [Locale] {
    static LOCALES: OnceLock<Vec<Locale>> = OnceLock::new();
    LOCALES.get_or_init(locales)
}

/// What the document's locale calls each locator kind, "p.", "pp.", "page",
/// "pages" for a page in English, "S." in German: the terms pandoc reads a
/// locator by. A term the locale lacks comes from its base language's locale,
/// then from English, as hayagriva looks terms up. The longest term comes
/// first, so "pp." is tried before "p.".
fn locator_terms(locales: &[Locale], code: &LocaleCode) -> Vec<(String, Locator)> {
    let lookup: Vec<&Locale> = [
        Some(code.clone()),
        code.fallback(),
        Some(LocaleCode::en_us()),
    ]
    .into_iter()
    .flatten()
    .filter_map(|lang| locales.iter().find(|l| l.lang.as_ref() == Some(&lang)))
    .collect();
    let mut terms = Vec::new();
    for kind in LOCATOR_KINDS {
        for form in [TermForm::Long, TermForm::Short] {
            let found = lookup
                .iter()
                .find_map(|l| l.term(Term::Locator(kind), form));
            let Some(term) = found else {
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

/// A style whose citations are footnotes, by the CSL `class` attribute, the
/// same test pandoc's citeproc applies.
fn is_note(style: &IndependentStyle) -> bool {
    style.settings.class == StyleClass::Note
}

/// What pandoc steps over to reach the letter it capitalises: opening
/// quotation marks, the straight one as escape_html() writes it, a bracket.
const BEFORE_FIRST_LETTER: [&str; 9] = ["&quot;", "'", "“", "‘", "„", "‚", "«", "‹", "("];

/// Pandoc capitalises the first letter of a note, past any tag and opening
/// mark; a note that opens with a digit or a dash stays as it is.
fn capitalise_first(html: &mut String) {
    let mut at = 0;
    loop {
        let rest = &html[at..];
        let skip = match rest.starts_with('<') {
            true => rest.find('>').map(|end| end + 1),
            false => BEFORE_FIRST_LETTER
                .iter()
                .find(|m| rest.starts_with(*m))
                .map(|m| m.len()),
        };
        match skip {
            Some(skip) => at += skip,
            None => break,
        }
    }
    if let Some(first) = html[at..].chars().next().filter(|c| c.is_lowercase()) {
        html.replace_range(at..at + first.len_utf8(), &first.to_uppercase().to_string());
    }
}

/// The bracket the style's layout opens a rendered citation with. A style
/// that brackets each number inside the layout, as ieee does, has none.
fn opening<'h>(html: &'h str, style: &IndependentStyle) -> Option<&'h str> {
    let own = style.citation.layout.prefix.as_deref()?;
    html.starts_with(own).then(|| &html[..own.len()])
}

fn closing<'h>(html: &'h str, style: &IndependentStyle) -> Option<&'h str> {
    let own = style.citation.layout.suffix.as_deref()?;
    html.ends_with(own).then(|| &html[html.len() - own.len()..])
}

/// Pandoc's extras hayagriva has no item for. A suppressed author cuts the
/// prose form "Doe (2020, p. 3)" at its bracket; the first item's prefix goes
/// in after the layout's opening bracket, the last item's suffix before its
/// closing one; without them, "see [1]" and "[1], emphasis added".
fn decorate(
    mut html: String,
    citation: &Citation,
    suffixes: &[&str],
    style: &IndependentStyle,
) -> String {
    let (Some(first), Some(suffix)) = (citation.items.first(), suffixes.last()) else {
        return html;
    };
    let bracket = style.citation.layout.prefix.as_deref().unwrap_or("(");
    if suppresses_author(citation, style) {
        if let Some(at) = html.find(bracket) {
            html.replace_range(..at, "");
        }
    }
    // A note style has no bracket; hayagriva opens the prose form with "("
    // all the same and ends it with the note's full stop.
    if suppresses_author(citation, style) && is_note(style) {
        if let Some(at) = html.find(bracket) {
            html.replace_range(at..at + bracket.len(), "");
        }
    }
    let prefix = first.prefix.trim();
    if !prefix.is_empty() {
        let at = opening(&html, style).map_or(0, str::len);
        html.insert_str(at, &format!("{} ", escape_html(prefix)));
    }
    let suffix = suffix.trim();
    if !suffix.is_empty() {
        let at = html.len() - closing(&html, style).map_or(0, str::len);
        html.insert_str(at, &format!(", {}", escape_html(suffix)));
    }
    // The in-text form of a note style prints its author in the sentence.
    if is_note(style) && !citation.in_text {
        capitalise_first(&mut html);
    }
    html
}

/// Pandoc's form of a citation inside a footnote in a note style: in
/// parentheses, without the note's final period.
fn in_parentheses(html: String) -> String {
    format!("({})", html.strip_suffix('.').unwrap_or(&html))
}

/// What follows a note's full stop: nothing, or the closing quotation mark
/// an American English locale moves it inside.
const AFTER_FULL_STOP: [&str; 3] = ["", "”", "’"];

/// Pandoc's form of the in-text citation `@key` in a note style: the note
/// inside the sentence, without its full stop.
fn without_full_stop(html: &str) -> String {
    for close in AFTER_FULL_STOP {
        if let Some(note) = html.strip_suffix(close).and_then(|h| h.strip_suffix('.')) {
            return format!("{note}{close}");
        }
    }
    html.to_string()
}

/// The affixes decorate() cannot reach: every prefix but the first item's goes
/// in front of its item, every suffix but the last item's after it. hayagriva
/// marks each item it prints with the item's index. False when an item with
/// such an affix was collapsed away, so its affix has no place.
fn place_inner_affixes(
    children: &mut ElemChildren,
    citation: &Citation,
    suffixes: &[&str],
) -> bool {
    let last = citation.items.len().saturating_sub(1);
    let text = |text: String| {
        ElemChild::Text(Formatted {
            text,
            formatting: Formatting::default(),
        })
    };
    let mut affixes: Vec<_> = citation
        .items
        .iter()
        .zip(suffixes)
        .enumerate()
        .map(|(i, (item, suffix))| {
            let (prefix, suffix) = (item.prefix.trim(), suffix.trim());
            (
                (i > 0 && !prefix.is_empty()).then(|| text(format!("{prefix} "))),
                (i < last && !suffix.is_empty()).then(|| text(format!(", {suffix}"))),
            )
        })
        .collect();
    let mut out = Vec::with_capacity(children.0.len());
    for child in children.0.drain(..) {
        let entry = match &child {
            ElemChild::Elem(Elem {
                meta: Some(ElemMeta::Entry(index)),
                ..
            }) => affixes.get_mut(*index),
            _ => None,
        };
        let Some((prefix, suffix)) = entry else {
            out.push(child);
            continue;
        };
        out.extend(prefix.take());
        out.push(child);
        out.extend(suffix.take());
    }
    children.0 = out;
    affixes
        .iter()
        .all(|(prefix, suffix)| prefix.is_none() && suffix.is_none())
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
    let _ = escaped(children).write_buf(&mut out, BufWriteFormat::Html);
    out
}

fn html_of(child: &ElemChild) -> String {
    let mut out = String::new();
    let _ = escaped_child(child).write_buf(&mut out, BufWriteFormat::Html);
    out
}

/// hayagriva writes a reference's text into the HTML as it is, so a "<" in a
/// title would open a tag. A copy with every text and URL escaped; the tags
/// hayagriva writes for its own formatting stay.
fn escaped(children: &ElemChildren) -> ElemChildren {
    ElemChildren(children.0.iter().map(escaped_child).collect())
}

fn escaped_child(child: &ElemChild) -> ElemChild {
    let formatted = |f: &Formatted| Formatted {
        text: escape_html(&f.text),
        formatting: f.formatting,
    };
    match child {
        ElemChild::Text(text) => ElemChild::Text(formatted(text)),
        ElemChild::Elem(elem) => ElemChild::Elem(Elem {
            children: escaped(&elem.children),
            display: elem.display,
            meta: elem.meta.clone(),
        }),
        ElemChild::Link { text, url } => ElemChild::Link {
            text: formatted(text),
            url: escape_html(url),
        },
        other => other.clone(),
    }
}

/// The four characters HTML gives a meaning, in text and in an attribute.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
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
    let locales = bundled_locales();
    let terms = locator_terms(locales, locale);
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
        // Pandoc prints the in-text form of a numeric style as the bare number,
        // that of a note style as the note.
        let prose = (citation.in_text && !is_numeric(style) && !is_note(style))
            || suppresses_author(citation, style);
        let items = items
            .iter()
            .map(|(reference, cited)| {
                let locator = split_locator(&cited.suffix, &terms).0;
                let item = CitationItem::with_locator(*reference, locator);
                match prose {
                    true => item.kind(CitePurpose::Prose),
                    false => item,
                }
            })
            .collect();
        let request = CitationRequest::new(items, style, Some(locale.clone()), locales, None);
        driver.citation(request);
    }
    let request = BibliographyRequest::new(style, Some(locale.clone()), locales);
    // hayagriva 0.10 panics in MLA's name disambiguation when one cited
    // work's author list is the start of another's; the panic stays on this
    // thread, and the citations stay as written.
    let Ok(rendered) = catch_unwind(AssertUnwindSafe(|| driver.finish(request))) else {
        out.warnings.push(
            "the citation library failed on these citations in this style; \
             they stay as written"
                .to_string(),
        );
        return out;
    };
    for ((index, _), mut citation) in cited.iter().zip(rendered.citations) {
        if !place_inner_affixes(
            &mut citation.citation,
            &citations[*index],
            &suffixes[*index],
        ) {
            out.warnings.push(format!(
                "citation {}: a prefix or suffix inside a collapsed group is not printed",
                index + 1
            ));
        }
        let text = html(&citation.citation);
        let citation = &citations[*index];
        let mut text = decorate(text, citation, &suffixes[*index], style);
        if citation.in_text && is_note(style) {
            text = without_full_stop(&text);
        } else if citation.in_note && is_note(style) {
            text = in_parentheses(text);
        }
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
    if header_truish(front_matter, "suppress-bibliography") {
        out.bibliography.clear();
    }
    out.warnings.splice(0..0, loaded.warnings);
    Ok(out)
}

/// Whether the header sets `key` to true as pandoc reads it: the boolean
/// `true`, or the text "t", "true" or "yes" in any case.
fn header_truish(front_matter: &str, key: &str) -> bool {
    let Ok(document) = yaml_serde::from_str::<yaml_serde::Value>(front_matter) else {
        return false;
    };
    match document.get(key) {
        Some(yaml_serde::Value::Bool(value)) => *value,
        Some(yaml_serde::Value::String(text)) => {
            ["t", "true", "yes"].contains(&text.trim().to_ascii_lowercase().as_str())
        }
        _ => false,
    }
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
            in_note: false,
        }
    }

    #[test]
    fn a_citation_inside_a_footnote_prints_in_parentheses_in_note_styles() {
        let references = read_biblatex(BIB, true).references;
        let mut noted = cite(&["doe2020"]);
        noted.items[0].suffix = "p. 40".into();
        noted.in_note = true;
        let mut prose = noted.clone();
        prose.in_text = true;
        let mut opening = noted.clone();
        opening.in_note = false;
        let cited = [opening, noted, prose];
        let render_in = |name| {
            let style = style_named(Some(name)).unwrap();
            render(&references, &style, &LocaleCode::en_us(), &cited).citations
        };
        let chicago = render_in("chicago-notes");
        assert!(chicago[0].as_deref().unwrap().ends_with("40."));
        assert_eq!(chicago[1].as_deref(), Some("(Doe and Roe, “A Study,” 40)"));
        assert!(!chicago[2].as_deref().unwrap().starts_with('('));
        let apa = render_in("apa");
        assert_eq!(apa[1].as_deref(), Some("(Doe &amp; Roe, 2020, p. 40)"));
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
                "(see Doe &amp; Roe, 2020, Chapter 2, emphasis added)",
                "(2020, p. 3)",
                "(Doe &amp; Roe, 2020, p. 12)"
            ]
        );
        assert_eq!(apa.warnings.len(), 1, "{:?}", apa.warnings);
        let ieee = render(
            &references,
            &style_named(Some("ieee")).unwrap(),
            &locale,
            &cited,
        );
        assert_eq!(
            ieee.citations[0].as_deref(),
            Some("see [1, Ch. 2], emphasis added")
        );
        assert_eq!(ieee.citations[1].as_deref(), Some("[1, p. 3]"));
    }

    #[test]
    fn in_text_form_prints_the_name_only_in_author_date_styles() {
        let references = read_biblatex(BIB, true).references;
        let mut prose = cite(&["doe2020"]);
        prose.in_text = true;
        let mut page = prose.clone();
        page.items[0].suffix = "p. 3".into();
        let cited = [prose, page];
        let texts = |name| {
            let style = style_named(Some(name)).unwrap();
            let out = render(&references, &style, &LocaleCode::en_us(), &cited);
            out.citations
                .into_iter()
                .map(Option::unwrap)
                .collect::<Vec<_>>()
        };
        assert_eq!(texts("ieee"), ["[1]", "[1, p. 3]"]);
        // Pandoc drops nature's locator too.
        let one = "<span style=\"vertical-align: super;\">1</span>";
        assert_eq!(texts("nature"), [one, one]);
        assert_eq!(
            texts("apa"),
            ["Doe &amp; Roe (2020)", "Doe &amp; Roe (2020, p. 3)"]
        );
    }

    #[test]
    fn ieee_prints_prefix_and_comment_outside_the_number_bracket() {
        let mut references = read_biblatex(BIB, true).references;
        references.extend(read_csl_yaml(ISSUE_EXAMPLE).unwrap().references);
        let mut see = cite(&["doe2020"]);
        see.items[0].prefix = "see".into();
        let mut page = cite(&["doe2020"]);
        page.items[0].suffix = "p. 3, emphasis added".into();
        let mut group = cite(&["doe2020", "ERA5LandHourlyData2019"]);
        group.items[0].prefix = "see".into();
        let style = style_named(Some("ieee")).unwrap();
        let cited = [see, page, group];
        let out = render(&references, &style, &LocaleCode::en_us(), &cited);
        assert_eq!(
            out.citations,
            [
                Some("see [1]".into()),
                Some("[1, p. 3], emphasis added".into()),
                Some("see [1], [2]".into())
            ]
        );
    }

    #[test]
    fn a_note_style_prints_the_note_without_an_opening_parenthesis() {
        let references = read_biblatex(BIB, true).references;
        let mut in_text = cite(&["doe2020"]);
        in_text.in_text = true;
        let mut year = cite(&["doe2020"]);
        year.items[0].suppress_author = true;
        let mut page = year.clone();
        page.items[0].suffix = "p. 3".into();
        let out = render(
            &references,
            &style_named(Some("chicago-notes")).unwrap(),
            &LocaleCode::en_us(),
            &[in_text, year, page],
        );
        let texts: Vec<_> = out
            .citations
            .iter()
            .map(|c| c.as_deref().unwrap())
            .collect();
        assert_eq!(
            texts,
            [
                "Jane Doe and Richard Roe, “A Study,” \
                 <span style=\"font-style: italic;\">J. Stud.</span> 3 (2020): 1–9",
                "“A Study.”",
                "“A Study,” 3."
            ]
        );
    }

    #[test]
    fn the_in_text_form_of_a_note_style_is_the_note_without_its_full_stop() {
        let references = read_biblatex(BIB, true).references;
        let mut first = cite(&["doe2020"]);
        first.in_text = true;
        let mut page = first.clone();
        page.items[0].suffix = "p. 3".into();
        let out = render(
            &references,
            &style_named(Some("chicago-shortened-notes")).unwrap(),
            &LocaleCode::en_us(),
            &[first, page],
        );
        assert_eq!(
            out.citations,
            [
                Some("Doe and Roe, “A Study”".to_string()),
                Some("Doe and Roe, “A Study,” 3".to_string())
            ]
        );
    }

    #[test]
    fn a_note_starts_with_a_capital_as_in_pandoc() {
        let mut references = read_biblatex(BIB, true).references;
        let berg = "references:\n- id: berg\n  title: Names\n  author:\n  - family: Berg\n    \
                    given: Ludwig\n    non-dropping-particle: van der\n";
        references.extend(read_csl_yaml(berg).unwrap().references);
        let prefixed = |prefix: &str, key: &str| {
            let mut citation = cite(&[key]);
            citation.items[0].prefix = prefix.into();
            citation
        };
        let mut prose = cite(&["berg"]);
        prose.in_text = true;
        let cited = [
            prefixed("see", "doe2020"),
            cite(&["berg"]),
            prefixed("\"see\"", "doe2020"),
            prefixed("2nd ed.", "doe2020"),
            prose,
        ];
        let locale = LocaleCode::en_us();
        let texts = |style: &str| {
            let style = style_named(Some(style)).unwrap();
            let out = render(&references, &style, &locale, &cited);
            out.citations.into_iter().flatten().collect::<Vec<_>>()
        };
        let notes = texts("chicago-shortened-notes");
        assert_eq!(
            notes[..4],
            [
                "See Doe and Roe, “A Study”.",
                "Van der Berg, <span style=\"font-style: italic;\">Names</span>.",
                "&quot;See&quot; Doe and Roe, “A Study”.",
                "2nd ed. Doe and Roe, “A Study”."
            ]
        );
        assert!(notes[4].starts_with("van der Berg"), "{}", notes[4]);
        assert_eq!(texts("chicago-author-date")[0], "(see Doe and Roe 2020)");
    }

    #[test]
    fn every_item_of_a_group_prints_its_prefix_and_suffix() {
        let mut references = read_biblatex(BIB, true).references;
        references.extend(read_csl_yaml(ISSUE_EXAMPLE).unwrap().references);
        let mut group = cite(&["ERA5LandHourlyData2019", "doe2020"]);
        group.items[0].prefix = "see".into();
        group.items[0].suffix = "first note".into();
        group.items[1].prefix = "also".into();
        group.items[1].suffix = "p. 3, last note".into();
        let cited = [group];
        let printed = |style: &str| {
            let out = render(
                &references,
                &style_named(Some(style)).unwrap(),
                &LocaleCode::en_us(),
                &cited,
            );
            assert!(out.warnings.is_empty(), "{:?}", out.warnings);
            out.citations[0].clone().unwrap()
        };
        assert_eq!(
            printed("apa"),
            "(see Copernicus Climate Change Service, 2019, first note; \
             also Doe &amp; Roe, 2020, p. 3, last note)"
        );
        assert_eq!(
            printed("chicago-author-date"),
            "(see Copernicus Climate Change Service 2019, first note; \
             also Doe and Roe 2020, 3, last note)"
        );
    }

    #[test]
    fn locator_terms_come_from_the_document_locale() {
        let references = read_biblatex(BIB, true).references;
        let texts = [
            "pp. 3-5, emphasis added",
            "Kap. 2",
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
            &locale_named(Some("de"), &style),
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
                "(Doe &amp; Roe, 2020, pp. 3-5, emphasis added)",
                "(Doe &amp; Roe, 2020, Kapitel 2)",
                "(Doe &amp; Roe, 2020, S. 7)",
                "(Doe &amp; Roe, 2020, page 9, see)",
                "(Doe &amp; Roe, 2020, nothing numeric)"
            ]
        );
        let style = style_named(Some("gb-7714-2015-note")).unwrap();
        let out = render(&references, &style, &locale_named(None, &style), &cited);
        let note = out.citations[0].as_deref().unwrap();
        assert!(
            note.ends_with("2020, 3: 1-9, pp. 3-5, emphasis added."),
            "{note}"
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

    #[test]
    fn suppress_bibliography_keeps_the_citations_and_drops_the_list() {
        let cited = [cite(&["ERA5LandHourlyData2019"])];
        for (setting, suppressed) in [
            ("true", true),
            ("\"Yes\"", true),
            ("t", true),
            ("false", false),
            ("no", false),
        ] {
            let header = format!("suppress-bibliography: {setting}\n{ISSUE_EXAMPLE}");
            let out = render_document(None, &header, false, &cited).unwrap();
            assert!(out.citations[0].is_some(), "{setting}");
            assert_eq!(out.bibliography.is_empty(), suppressed, "{setting}");
        }
    }

    #[test]
    fn a_failing_style_warns_and_leaves_the_citations_as_written() {
        // Two works whose shorter author list is the start of the longer one
        // make hayagriva 0.10's MLA disambiguation index past the list.
        let refs = read_csl_yaml(
            "- id: a2012\n  type: article-journal\n  title: Forcing\n  issued: 2012\n  author:\n  \
             - family: Andrews\n    given: Timothy\n  - family: Gregory\n    given: Jonathan\n  \
             - family: Webb\n    given: Mark\n  - family: Taylor\n    given: Karl\n\
             - id: a2015\n  type: article-journal\n  title: Feedback\n  issued: 2015\n  author:\n  \
             - family: Andrews\n    given: Timothy\n  - family: Gregory\n    given: Jonathan\n  \
             - family: Webb\n    given: Mark\n",
        )
        .unwrap();
        let style = style_named(Some("modern-language-association")).unwrap();
        let locale = locale_named(None, &style);
        let out = render(
            &refs.references,
            &style,
            &locale,
            &[cite(&["a2012"]), cite(&["a2015"])],
        );
        assert_eq!(out.citations, vec![None, None]);
        assert!(out.bibliography.is_empty());
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("failed"));
    }

    /// The header of a fixture document, between its `---` lines.
    /// The header as the app's `frontMatter()` takes it, for LF and CRLF files.
    fn front_matter(text: &str) -> &str {
        let (open, close) = if text.starts_with("---\r\n") {
            ("---\r\n", "\r\n---\r\n")
        } else {
            ("---\n", "\n---\n")
        };
        let body = text.strip_prefix(open).unwrap_or("");
        body.split(close).next().unwrap_or("")
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

    /// Every bundled style with several locales, among them an unknown and
    /// an empty one, renders the test references without a panic: hayagriva
    /// panics on a date range, which the reader removes, and this sweep is
    /// the net under the rest.
    #[test]
    fn every_bundled_style_renders_in_every_locale_without_a_panic() {
        let mut references = read_biblatex(BIB, true).references;
        references.extend(read_csl_yaml(ISSUE_EXAMPLE).unwrap().references);
        let mut seen = cite(&["doe2020"]);
        seen.items[0].prefix = "see".into();
        seen.items[0].suffix = "p. 3, emphasis added".into();
        let mut year = cite(&["doe2020"]);
        year.items[0].suppress_author = true;
        let mut prose = cite(&["ERA5LandHourlyData2019"]);
        prose.in_text = true;
        let group = cite(&["doe2020", "ERA5LandHourlyData2019", "nobody"]);
        let cited = [seen, year, prose, group];
        let mut styles = 0;
        for archived in hayagriva::archive::ArchivedStyle::all() {
            let Style::Independent(style) = archived.get() else {
                continue;
            };
            styles += 1;
            for lang in [None, Some("en-GB"), Some("de-DE"), Some("xx"), Some("")] {
                let locale = locale_named(lang, &style);
                let out = render(&references, &style, &locale, &cited);
                assert_eq!(out.citations.len(), 4, "{}", archived.names()[0]);
                assert!(
                    out.citations[..3].iter().all(Option::is_some),
                    "{} {lang:?}: {:?}",
                    archived.names()[0],
                    out.citations
                );
                // hayagriva's own alphanumeric style has no bibliography layout.
                if style.bibliography.is_some() {
                    assert_eq!(out.bibliography.len(), 2, "{}", archived.names()[0]);
                }
            }
        }
        assert_eq!(styles, 80);
    }

    #[test]
    fn the_catalogue_lists_every_bundled_style_and_locale() {
        let found = catalogue();
        assert_eq!(found.styles.len(), 80);
        assert!(found.styles.contains(&"apa".to_string()));
        assert!(found.locales.contains(&"de-DE".to_string()));
        assert!(found.locales.len() > 40, "{}", found.locales.len());
    }

    #[test]
    fn markup_in_a_reference_prints_as_text() {
        let bib = "@article{odd, author={Doe, Jane}, title={A <b>bold</b> & co}, \
                   journal={J}, year={2020}, doi={10.1/a\"b}}";
        let references = read_biblatex(bib, true).references;
        let style = style_named(Some("apa")).unwrap();
        let mut cited = cite(&["odd"]);
        cited.items[0].prefix = "<i>see</i>".into();
        cited.items[0].suffix = "p. 3, <u>really</u>".into();
        let out = render(&references, &style, &LocaleCode::en_us(), &[cited]);
        assert_eq!(
            out.citations[0].as_deref(),
            Some("(&lt;i&gt;see&lt;/i&gt; Doe, 2020, p. 3, &lt;u&gt;really&lt;/u&gt;)")
        );
        let html = &out.bibliography[0].html;
        assert!(
            html.contains("A &lt;b&gt;bold&lt;/b&gt; &amp; co"),
            "{html}"
        );
        assert!(
            html.contains("href=\"https://doi.org/10.1/a&quot;b\""),
            "{html}"
        );
        assert!(!html.contains("<b>"), "{html}");
    }
}
