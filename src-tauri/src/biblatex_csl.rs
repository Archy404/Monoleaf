//! A `.bib` entry as a CSL item. The entry types and fields map as Pandoc's
//! BibTeX reader maps them, so a `.bib` file renders like the same works in
//! CSL JSON, and like Pandoc prints them. A field Pandoc does not read is not
//! read here either.

use biblatex::{Chunk, Chunks, ChunksExt, ChunksRef, Date, DateValue as BibDate, Entry, EntryType};
use hayagriva::citationberg::json::{
    DateValue, Item, LiteralName, NameItem, NameValue, Value, VecDate, VecDateRange,
};
use std::collections::BTreeMap;

/// The genre Pandoc gives a thesis without a `type` field.
const PHD_THESIS: &str = "PhD thesis";
const MASTERS_THESIS: &str = "Master's thesis";

/// BibLaTeX localisation keys with their English text from BibLaTeX's
/// `english.lbx`, as Pandoc prints them; any other value is its own text.
const KEY_TEXTS: [(&str, &str); 34] = [
    ("bathesis", "Bachelor's thesis"),
    ("mathesis", MASTERS_THESIS),
    ("phdthesis", PHD_THESIS),
    ("candthesis", "Candidate thesis"),
    ("resreport", "research report"),
    ("techreport", "technical report"),
    ("software", "computer software"),
    ("datacd", "CD-ROM"),
    ("audiocd", "audio CD"),
    ("inpreparation", "in preparation"),
    ("submitted", "submitted"),
    ("forthcoming", "forthcoming"),
    ("inpress", "in press"),
    ("prepublished", "pre-published"),
    ("countryde", "Germany"),
    ("countryeu", "European Union"),
    ("countryep", "European Union"),
    ("countryfr", "France"),
    ("countryuk", "United Kingdom"),
    ("countryus", "United States of America"),
    ("patent", "patent"),
    ("patentde", "German patent"),
    ("patenteu", "European patent"),
    ("patentfr", "French patent"),
    ("patentuk", "British patent"),
    ("patentus", "U.S. patent"),
    ("patreq", "patent request"),
    ("patreqde", "German patent request"),
    ("patreqeu", "European patent request"),
    ("patreqfr", "French patent request"),
    ("patrequk", "British patent request"),
    ("patrequs", "U.S. patent request"),
    ("newseries", "new series"),
    ("oldseries", "old series"),
];

/// The start of the command that prints a key's text, as the parser keeps it.
const BIBSTRING: &str = "\\bibstring{";

/// The `pubstate` texts a `year` field may hold instead of a year.
const STATUS_TEXTS: [&str; 5] = [
    "in press",
    "forthcoming",
    "submitted",
    "in preparation",
    "pre-published",
];

/// BibLaTeX language names and the CSL locale each stands for.
const LANGUAGES: [(&str, &str); 21] = [
    ("english", "en-US"),
    ("american", "en-US"),
    ("usenglish", "en-US"),
    ("british", "en-GB"),
    ("ukenglish", "en-GB"),
    ("australian", "en-GB"),
    ("newzealand", "en-GB"),
    ("german", "de-DE"),
    ("ngerman", "de-DE"),
    ("french", "fr-FR"),
    ("spanish", "es-ES"),
    ("italian", "it-IT"),
    ("dutch", "nl-NL"),
    ("portuguese", "pt-PT"),
    ("brazilian", "pt-BR"),
    ("russian", "ru-RU"),
    ("swedish", "sv-SE"),
    ("danish", "da-DK"),
    ("norwegian", "nb-NO"),
    ("finnish", "fi-FI"),
    ("polish", "pl-PL"),
];

/// The `eprinttype` values that name a site, and the URL an eprint id follows.
const EPRINT_SITES: [(&str, &str); 4] = [
    ("arxiv", "https://arxiv.org/abs/"),
    ("jstor", "https://www.jstor.org/stable/"),
    ("pubmed", "https://www.ncbi.nlm.nih.gov/pubmed/"),
    ("googlebooks", "https://books.google.com?id="),
];

/// The CSL item for one entry. Never fails: a field that cannot be read as
/// its type is taken as text, a date that is no date becomes the literal
/// text of the date. `english_document` says whether the citing document is
/// English; only then do titles go to sentence case, as in Pandoc.
pub fn item_of(entry: &Entry, english_document: bool) -> Item {
    let mut item = Fields::default();
    item.set("id", Some(entry.key.clone()));
    item.set("type", csl_type(entry).map(str::to_string));
    item.set("genre", genre(entry));
    titles(entry, &mut item, english_document);
    numbers(entry, &mut item);
    publishers(entry, &mut item);
    identifiers(entry, &mut item);
    notes(entry, &mut item);
    names(entry, &mut item);
    dates(entry, &mut item);
    Item(item.0)
}

/// The variables of an item while it is built; `set` with `None` sets nothing.
#[derive(Default)]
struct Fields(BTreeMap<String, Value>);

impl Fields {
    fn set(&mut self, name: &str, value: Option<String>) {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            self.0.insert(name.to_string(), Value::String(value));
        }
    }

    fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }
}

/// The CSL item type of the entry type; `None` for `misc`, `set`, `xdata`
/// and unknown types, which Pandoc leaves without a type.
fn csl_type(entry: &Entry) -> Option<&'static str> {
    use EntryType::*;
    Some(match &entry.entry_type {
        Article | Periodical | SuppPeriodical => match text(entry, "entrysubtype").as_deref() {
            Some("magazine") => "article-magazine",
            Some("newspaper") => "article-newspaper",
            _ => "article-journal",
        },
        Book | BookInBook | Collection | Manual | MvBook | MvCollection | MvProceedings
        | MvReference | Proceedings | Reference => "book",
        Booklet => "pamphlet",
        Dataset => "dataset",
        Online => "webpage",
        InBook | InCollection | SuppBook | SuppCollection => "chapter",
        InReference => "entry-encyclopedia",
        InProceedings => "paper-conference",
        MastersThesis | PhdThesis | Thesis => "thesis",
        Patent => "patent",
        Report | TechReport => "report",
        Software => "software",
        Unpublished => {
            if ["eventdate", "eventtitle", "venue"]
                .iter()
                .any(|f| entry.get(f).is_some())
            {
                "speech"
            } else {
                "manuscript"
            }
        }
        Misc | Set | XData => return None,
        Unknown(name) => match name.to_ascii_lowercase().as_str() {
            "electronic" | "www" => "webpage",
            "artwork" | "image" => "graphic",
            "audio" | "music" => "song",
            "jurisdiction" => "legal_case",
            "legislation" | "standard" => "legislation",
            "legal" => "treaty",
            "letter" | "letters" => "personal_communication",
            "movie" | "video" => "motion_picture",
            "performance" => "speech",
            "review" => "review",
            "newsarticle" => "article-newspaper",
            "commentary" => "book",
            "data" => "dataset",
            _ => return None,
        },
    })
}

/// A part of a book: its `booktitle` is the container.
fn is_chapter_like(entry: &Entry) -> bool {
    use EntryType::*;
    matches!(
        entry.entry_type,
        InBook
            | InCollection
            | InProceedings
            | InReference
            | BookInBook
            | SuppBook
            | SuppCollection
    )
}

/// A periodical article: its `number` is the issue.
fn is_article_like(entry: &Entry) -> bool {
    use EntryType::*;
    match &entry.entry_type {
        Article | Periodical | SuppPeriodical => true,
        Unknown(name) => name.eq_ignore_ascii_case("review"),
        _ => false,
    }
}

/// A book or a part of one: its `number` is the number in the series.
fn is_book_like(entry: &Entry) -> bool {
    use EntryType::*;
    matches!(
        entry.entry_type,
        Book | Collection
            | Proceedings
            | Reference
            | MvBook
            | MvCollection
            | MvProceedings
            | MvReference
    ) || is_chapter_like(entry)
}

/// The `type` field as text, or the thesis kind the entry type implies.
fn genre(entry: &Entry) -> Option<String> {
    match text(entry, "type") {
        Some(value) => Some(key_text(&value)),
        None => match entry.entry_type {
            EntryType::PhdThesis => Some(PHD_THESIS.to_string()),
            EntryType::MastersThesis => Some(MASTERS_THESIS.to_string()),
            _ => None,
        },
    }
}

/// A BibLaTeX key's English text; any other value is its own text.
fn key_text(value: &str) -> String {
    let key = value.to_ascii_lowercase();
    KEY_TEXTS
        .iter()
        .find(|(name, _)| *name == key)
        .map_or_else(|| value.to_string(), |(_, text)| text.to_string())
}

/// Text with every word that is a BibLaTeX key replaced by the key's text.
fn keys_resolved(value: &str) -> String {
    value.split(' ').map(key_text).collect::<Vec<_>>().join(" ")
}

/// Text with every `\bibstring{key}` replaced by the key's text.
fn bibstrings_resolved(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some((before, after)) = rest.split_once(BIBSTRING) {
        let Some((key, tail)) = after.split_once('}') else {
            break;
        };
        out.push_str(before);
        out.push_str(&key_text(key));
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Title, container and series. A book with a `maintitle` is a volume of
/// it: the main title is the title, the book's own becomes the volume title.
/// A chapter of such a volume is in the main title, its book is the volume.
/// Titles of an English entry go to sentence case, as Pandoc puts them, so a
/// style may raise them again; a journal's name keeps its case. A periodical
/// is an issue of a journal: its `title` names the journal, and its own title
/// is the `issuetitle` where it has one.
fn titles(entry: &Entry, item: &mut Fields, english_document: bool) {
    let english = english_document && is_english(entry);
    let periodical = entry.entry_type == EntryType::Periodical;
    let issue_or = |issue_field: &'static str, field: &'static str| {
        if periodical && entry.get(issue_field).is_some() {
            issue_field
        } else {
            field
        }
    };
    let subtitle = issue_or("issuesubtitle", "subtitle");
    let own = joined(
        entry,
        issue_or("issuetitle", "title"),
        subtitle,
        "titleaddon",
        english,
    );
    if !is_chapter_like(entry) && entry.get("maintitle").is_some() {
        item.set(
            "title",
            joined(
                entry,
                "maintitle",
                "mainsubtitle",
                "maintitleaddon",
                english,
            ),
        );
        item.set("volume-title", own);
    } else {
        item.set("title", own.clone());
        let short = match titled(entry, "shorttitle", english) {
            Some(short) => Some(short),
            None => {
                let title = titled(entry, "title", english);
                let has_more = entry.get(subtitle).is_some()
                    || entry.get("titleaddon").is_some()
                    || title.as_deref().is_some_and(|t| t.contains(':'));
                has_more.then(|| title.map(|t| before_colon(&t))).flatten()
            }
        };
        item.set("title-short", short);
    }
    if periodical {
        item.set(
            "container-title",
            joined(entry, "title", "subtitle", "titleaddon", false),
        );
    } else if is_chapter_like(entry) && entry.get("maintitle").is_some() {
        item.set(
            "container-title",
            joined(
                entry,
                "maintitle",
                "mainsubtitle",
                "maintitleaddon",
                english,
            ),
        );
        item.set(
            "volume-title",
            joined(
                entry,
                "booktitle",
                "booksubtitle",
                "booktitleaddon",
                english,
            ),
        );
    } else if is_chapter_like(entry) && entry.get("booktitle").is_some() {
        item.set(
            "container-title",
            joined(
                entry,
                "booktitle",
                "booksubtitle",
                "booktitleaddon",
                english,
            ),
        );
    } else if entry.get("journaltitle").is_some() {
        item.set(
            "container-title",
            joined(
                entry,
                "journaltitle",
                "journalsubtitle",
                "journaltitleaddon",
                false,
            ),
        );
    } else {
        item.set("container-title", text(entry, "journal"));
    }
    item.set(
        "container-title-short",
        text(entry, "shorttitle")
            .filter(|_| periodical)
            .or_else(|| text(entry, "shortjournal")),
    );
    item.set("collection-title", series(entry, english));
    item.set("event", titled(entry, "eventtitle", english));
    item.set("original-title", titled(entry, "origtitle", english));
}

/// The series. A number is the count of a journal's series and becomes
/// "3rd series", in English whatever the document's language, as Pandoc
/// writes it; any other value is a title.
fn series(entry: &Entry, english: bool) -> Option<String> {
    match text(entry, "series") {
        Some(number) if number.bytes().all(|b| b.is_ascii_digit()) => {
            Some(format!("{} series", ordinal(&number)))
        }
        _ => titled(entry, "series", english),
    }
}

/// A number with its English ordinal suffix: "st", "nd" and "rd" after a
/// last digit 1, 2 and 3, except in 11, 12 and 13; "th" otherwise.
fn ordinal(number: &str) -> String {
    let suffix = match number.as_bytes() {
        [.., b'1', b'1' | b'2' | b'3'] => "th",
        [.., b'1'] => "st",
        [.., b'2'] => "nd",
        [.., b'3'] => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

/// The part of a title before its first colon, the whole title without one.
fn before_colon(title: &str) -> String {
    title
        .split(':')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Whether the entry's titles are English: no `langid`, or one that names an
/// English variant. Pandoc puts only those into sentence case.
fn is_english(entry: &Entry) -> bool {
    match text(entry, "langid").or_else(|| text(entry, "hyphenation")) {
        None => true,
        Some(name) => language_code(&name).starts_with("en"),
    }
}

/// The CSL locale of a BibLaTeX language name; an unknown name as it is.
fn language_code(name: &str) -> String {
    let key = name.to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|(bib, _)| *bib == key)
        .map_or_else(|| name.to_string(), |(_, code)| code.to_string())
}

/// "Title: Subtitle. Addon", each part only when present; after a part that
/// ends in punctuation only a space follows.
fn joined(
    entry: &Entry,
    title: &str,
    subtitle: &str,
    addon: &str,
    sentence_case: bool,
) -> Option<String> {
    let mut out = titled(entry, title, sentence_case)?;
    for (field, separator) in [(subtitle, ":"), (addon, ".")] {
        if let Some(part) = titled(entry, field, sentence_case) {
            if !out.ends_with(['!', '?', '.', ',', ':', ';']) {
                out.push_str(separator);
            }
            out.push(' ');
            out.push_str(&part);
        }
    }
    Some(out)
}

/// Pages, volume, edition and the `number` field, which is the issue of an
/// article, the number in a series of a book, and a plain number elsewhere.
fn numbers(entry: &Entry, item: &mut Fields) {
    item.set(
        "page",
        text(entry, "pages").map(|p| p.replace('\u{2013}', "-")),
    );
    let volume = text(entry, "volume");
    let part = text(entry, "part");
    item.set(
        "volume",
        match (volume, part) {
            (Some(volume), Some(part)) => Some(format!("{volume}.{part}")),
            (volume, part) => volume.or(part),
        },
    );
    item.set("number-of-volumes", text(entry, "volumes"));
    item.set("number-of-pages", text(entry, "pagetotal"));
    item.set("chapter-number", text(entry, "chapter"));
    item.set("edition", text(entry, "edition"));
    item.set("version", text(entry, "version"));
    let number = text(entry, "number");
    if is_article_like(entry) {
        let issue = text(entry, "issue");
        item.set(
            "issue",
            match (number, issue) {
                (Some(number), Some(issue)) => Some(format!("{number}, {issue}")),
                (number, _) => number,
            },
        );
    } else if is_book_like(entry) {
        item.set("collection-number", number);
    } else {
        item.set("number", number);
    }
}

/// Publisher and places. School, institution, organization, `howpublished`
/// and publisher are all the publisher, in that order; a `location` of a
/// patent is its jurisdiction.
fn publishers(entry: &Entry, item: &mut Fields) {
    let mut parts = Vec::new();
    for field in ["school", "institution", "organization"] {
        parts.extend(list(entry, field));
    }
    parts.extend(text(entry, "howpublished"));
    parts.extend(list(entry, "publisher"));
    item.set("publisher", (!parts.is_empty()).then(|| parts.join("; ")));
    let is_patent = entry.entry_type == EntryType::Patent;
    let location = listed(entry, "location");
    let address = listed(entry, "address");
    if is_patent {
        item.set("publisher-place", address);
        item.set("jurisdiction", location.map(|l| keys_resolved(&l)));
    } else {
        item.set("publisher-place", address.or(location));
    }
    item.set("original-publisher", text(entry, "origpublisher"));
    item.set("original-publisher-place", listed(entry, "origlocation"));
    item.set("event-place", text(entry, "venue"));
}

/// URL, DOI and the identifier fields, which CSL names in capitals; an
/// eprint on a known site gives the URL when the entry has none.
fn identifiers(entry: &Entry, item: &mut Fields) {
    // The options `url=false` and `doi=false` leave the field out, though an
    // online entry keeps its URL, as in BibLaTeX.
    if entry.entry_type == EntryType::Online || !has_option(entry, "url", "false") {
        item.set("URL", verbatim(entry, "url"));
    }
    if !item.has("URL") {
        let site = verbatim(entry, "eprinttype")
            .or_else(|| verbatim(entry, "archiveprefix"))
            .map(|s| s.to_ascii_lowercase());
        if let (Some(site), Some(id)) = (site, verbatim(entry, "eprint")) {
            if let Some((_, base)) = EPRINT_SITES.iter().find(|(name, _)| *name == site) {
                item.set("URL", Some(format!("{base}{id}")));
            }
        }
    }
    if !has_option(entry, "doi", "false") {
        item.set("DOI", verbatim(entry, "doi"));
    }
    item.set("ISBN", verbatim(entry, "isbn"));
    item.set("ISSN", verbatim(entry, "issn"));
    item.set("PMID", verbatim(entry, "pmid"));
    item.set("PMCID", verbatim(entry, "pmcid"));
    item.set("call-number", text(entry, "library"));
}

/// Note, abstract, keywords, status and language. The note of a periodical
/// is not read, as Pandoc does not read it.
fn notes(entry: &Entry, item: &mut Fields) {
    item.set(
        "annote",
        text(entry, "annotation").or_else(|| text(entry, "annote")),
    );
    item.set("abstract", text(entry, "abstract"));
    item.set("keyword", text(entry, "keywords"));
    let note = (entry.entry_type != EntryType::Periodical)
        .then(|| text(entry, "note"))
        .flatten();
    let addendum = text(entry, "addendum");
    item.set(
        "note",
        match (note, addendum) {
            (Some(note), Some(addendum)) => Some(joined_sentences(note, &addendum)),
            (note, addendum) => note.or(addendum),
        },
    );
    item.set("status", text(entry, "pubstate").map(|s| key_text(&s)));
    // Pandoc keeps "in press" as the literal text of the date and the style
    // prints it there; hayagriva prints no literal date, so the status says it.
    if !item.has("status") && entry.get("date").is_none() {
        if let Some(literal) = text(entry, "year") {
            let known = STATUS_TEXTS
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&literal));
            item.set("status", known.then_some(literal));
        }
    }
    let language = text(entry, "langid").or_else(|| text(entry, "hyphenation"));
    item.set("language", language.map(|name| language_code(&name)));
}

/// "First. Second", with only a space when the first already ends a sentence.
fn joined_sentences(mut first: String, second: &str) -> String {
    if !first.ends_with(['!', '?', '.', ',', ':', ';']) {
        first.push('.');
    }
    first.push(' ');
    first.push_str(second);
    first
}

/// Author, editor, translator and the author of the book a chapter is in.
/// An `editortype` of director makes the editors directors.
fn names(entry: &Entry, item: &mut Fields) {
    let editor_variable = match text(entry, "editortype").as_deref() {
        Some("director") => "director",
        _ => "editor",
    };
    let use_prefix = has_option(entry, "useprefix", "true");
    for (field, variable) in [
        ("author", "author"),
        ("editor", editor_variable),
        ("director", "director"),
        ("translator", "translator"),
        ("bookauthor", "container-author"),
    ] {
        let Ok(persons) = entry.get_as::<Vec<biblatex::Person>>(field) else {
            continue;
        };
        let values: Vec<NameValue> = persons
            .iter()
            .map(|person| name_value(person, use_prefix))
            .collect();
        if !values.is_empty() {
            item.0.insert(variable.to_string(), Value::Names(values));
        }
    }
}

/// Whether the entry's `options` field, a comma-separated list, gives the
/// option the value; `useprefix` alone is `useprefix=true`.
fn has_option(entry: &Entry, option: &str, wanted: &str) -> bool {
    text(entry, "options").is_some_and(|options| {
        options.split(',').any(|setting| {
            let mut parts = setting.splitn(2, '=');
            let key = parts.next().unwrap_or_default().trim();
            let value = parts.next().map_or("true", str::trim);
            key.eq_ignore_ascii_case(option) && value.eq_ignore_ascii_case(wanted)
        })
    })
}

/// One name. A name the parser could not split, "{Copernicus Climate Change
/// Service}", is a literal name. A particle is a dropping particle, as
/// Pandoc takes it, unless the name or the entry's options say `useprefix`.
fn name_value(person: &biblatex::Person, use_prefix: bool) -> NameValue {
    let family = unlatex(&person.name);
    let given = unlatex(&person.given_name);
    let particle = unlatex(&person.prefix);
    let suffix = unlatex(&person.suffix);
    if given.is_empty()
        && particle.is_empty()
        && suffix.is_empty()
        && family.contains(char::is_whitespace)
    {
        return NameValue::Literal(LiteralName { literal: family });
    }
    let (non_dropping, dropping) = match person.use_prefix.unwrap_or(use_prefix) {
        true => (Some(particle), None),
        false => (None, Some(particle)),
    };
    NameValue::Item(NameItem {
        family,
        given: (!given.is_empty()).then(|| spaced_initials(&given)),
        non_dropping_particle: non_dropping.filter(|p| !p.is_empty()),
        dropping_particle: dropping.filter(|p| !p.is_empty()),
        suffix: (!suffix.is_empty()).then_some(suffix),
        comma_suffix: None,
    })
}

/// "J.G." as "J. G.": a space after an initial's period when a capital
/// follows directly.
fn spaced_initials(given: &str) -> String {
    let mut out = String::with_capacity(given.len() + 2);
    let mut chars = given.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '.' && chars.peek().is_some_and(|next| next.is_uppercase()) {
            out.push(' ');
        }
    }
    out
}

/// The date fields: `date` or year, month and day; the original, URL and
/// event dates likewise. A date that is no date keeps its text as the
/// literal, so "in press" is not lost.
fn dates(entry: &Entry, item: &mut Fields) {
    for (prefix, variable) in [
        ("", "issued"),
        ("orig", "original-date"),
        ("url", "accessed"),
        ("event", "event-date"),
    ] {
        let field = format!("{prefix}date");
        let year = format!("{prefix}year");
        let date = match entry.get(&field) {
            Some(chunks) => Date::parse(chunks)
                .map(Some)
                .unwrap_or_else(|_| None)
                .map(Ok)
                .unwrap_or_else(|| Err(plain(chunks))),
            None => match entry.get(&year) {
                Some(chunks) => Date::parse_three_fields(
                    chunks,
                    entry.get(&format!("{prefix}month")),
                    entry.get(&format!("{prefix}day")),
                )
                .map_err(|_| plain(chunks)),
                None => continue,
            },
        };
        let value = match date {
            Ok(date) => date_parts(&date),
            Err(literal) => DateValue::DateParts {
                date_parts: VecDateRange(Vec::new()),
                literal: Some(literal),
                season: None,
                circa: false,
            },
        };
        item.0.insert(variable.to_string(), Value::Date(value));
    }
}

/// A parsed date as CSL date parts: year, then month and day where given.
fn date_parts(date: &Date) -> DateValue {
    let parts = |d: &biblatex::Datetime| {
        let mut parts = vec![d.year as i16];
        if let Some(month) = d.month {
            // The parser counts months and days from zero, CSL from one.
            parts.push(i16::from(month) + 1);
            if let Some(day) = d.day {
                parts.push(i16::from(day) + 1);
            }
        }
        VecDate(parts)
    };
    let range = match &date.value {
        BibDate::At(d) | BibDate::After(d) | BibDate::Before(d) => vec![parts(d)],
        BibDate::Between(start, end) => vec![parts(start), parts(end)],
    };
    DateValue::DateParts {
        date_parts: VecDateRange(range),
        literal: None,
        season: None,
        circa: date.approximate || date.uncertain,
    }
}

/// A field's text, or `None` when the entry has no such field or it is empty.
fn text(entry: &Entry, field: &str) -> Option<String> {
    let value = plain(entry.get(field)?);
    (!value.is_empty()).then_some(value)
}

/// A title field's text, in sentence case when asked.
fn titled(entry: &Entry, field: &str, sentence_case: bool) -> Option<String> {
    if !sentence_case {
        return text(entry, field);
    }
    let value = unlatex(&sentence_case_of(entry.get(field)?));
    (!value.is_empty()).then_some(value)
}

/// A title in sentence case, by the rule Pandoc applies: a word is lowered
/// only when its first letter is its only capital, so "NASA", "McDonald" and
/// "iPhone" stay; the first word, and the first after a ":", "?" or "!"
/// that ends a word (not the one in "a:b" or "http://"), gets a capital
/// unless it has one of its own, as "eBay"; a hyphen ends a word; what
/// braces protect stays as written.
fn sentence_case_of(chunks: ChunksRef) -> String {
    let mut out = String::new();
    let mut sentence_start = true;
    for chunk in chunks {
        match &chunk.v {
            Chunk::Normal(text) => {
                let text = bibstrings_resolved(text);
                let mut word = String::new();
                let mut chars = text.chars().peekable();
                while let Some(c) = chars.next() {
                    if c.is_alphanumeric() || c == '\'' || c == '\u{2019}' {
                        word.push(c);
                        continue;
                    }
                    flush_word(&mut word, &mut out, &mut sentence_start);
                    out.push(c);
                    let ends_word = chars.peek().is_none_or(|next| next.is_whitespace());
                    if matches!(c, ':' | '?' | '!') && ends_word {
                        sentence_start = true;
                    }
                }
                flush_word(&mut word, &mut out, &mut sentence_start);
            }
            Chunk::Verbatim(text) => {
                out.push_str(text);
                sentence_start = false;
            }
            Chunk::Math(text) => {
                out.push('$');
                out.push_str(text);
                out.push('$');
                sentence_start = false;
            }
        }
    }
    out
}

/// Appends the word in its sentence-case form and clears it: capitalised at
/// a sentence start when it has no capital, lowered when its first letter is
/// its only capital, otherwise as written.
fn flush_word(word: &mut String, out: &mut String, sentence_start: &mut bool) {
    if word.is_empty() {
        return;
    }
    let mut chars = word.chars();
    let first_is_capital = chars.next().is_some_and(char::is_uppercase);
    let rest_has_capital = chars.any(char::is_uppercase);
    if *sentence_start && !first_is_capital && !rest_has_capital {
        let mut chars = word.chars();
        out.extend(chars.next().into_iter().flat_map(char::to_uppercase));
        out.extend(chars);
    } else if !*sentence_start && first_is_capital && !rest_has_capital {
        out.push_str(&word.to_lowercase());
    } else {
        out.push_str(word);
    }
    word.clear();
    *sentence_start = false;
}

/// An identifier field as written: a URL or DOI keeps its "~" and "'".
fn verbatim(entry: &Entry, field: &str) -> Option<String> {
    let value = entry.get(field)?.format_verbatim().trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// A field that BibLaTeX lists with `and`, as its parts.
fn list(entry: &Entry, field: &str) -> Vec<String> {
    match entry.get_as::<Vec<Chunks>>(field) {
        Ok(parts) => parts
            .iter()
            .map(|chunks| plain(chunks))
            .filter(|part| !part.is_empty())
            .collect(),
        Err(_) => text(entry, field).into_iter().collect(),
    }
}

/// A list field's parts joined with "; ", or `None` when it has none.
fn listed(entry: &Entry, field: &str) -> Option<String> {
    let parts = list(entry, field);
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Chunks as plain text: braces gone, what LaTeX writes as text resolved.
fn plain(chunks: ChunksRef) -> String {
    unlatex(&bibstrings_resolved(&chunks.format_verbatim()))
}

/// Text with TeX's conventions resolved: "--" an en dash, "---" an em dash,
/// "~" a no-break space, an escaped character the character itself, quotes
/// and apostrophes the typographic ones.
fn unlatex(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' if chars.peek() == Some(&'`') => {
                chars.next();
                out.push('\u{201c}');
            }
            '\'' if chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\u{201d}');
            }
            '\'' => out.push('\u{2019}'),
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                if chars.peek() == Some(&'-') {
                    chars.next();
                    out.push('\u{2014}');
                } else {
                    out.push('\u{2013}');
                }
            }
            '~' => out.push('\u{a0}'),
            '\\' if chars.peek().is_some_and(|next| "&%$#_{}".contains(*next)) => {
                out.push(chars.next().unwrap_or_default());
            }
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(bib: &str) -> Item {
        let bibliography = biblatex::Bibliography::parse(bib).unwrap();
        item_of(bibliography.iter().next().unwrap(), true)
    }

    fn string<'a>(item: &'a Item, name: &str) -> Option<&'a str> {
        match item.0.get(name) {
            Some(Value::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Names as "family|given|non-dropping|dropping|suffix", a literal in braces.
    fn names(item: &Item, variable: &str) -> Vec<String> {
        let Some(Value::Names(names)) = item.0.get(variable) else {
            return Vec::new();
        };
        names
            .iter()
            .map(|name| match name {
                NameValue::Literal(l) => format!("{{{}}}", l.literal),
                NameValue::Item(n) => [
                    n.family.as_str(),
                    n.given.as_deref().unwrap_or_default(),
                    n.non_dropping_particle.as_deref().unwrap_or_default(),
                    n.dropping_particle.as_deref().unwrap_or_default(),
                    n.suffix.as_deref().unwrap_or_default(),
                ]
                .join("|"),
            })
            .collect()
    }

    /// A date as its parts and its literal: "[[2019, 3]] in press".
    fn date(item: &Item, variable: &str) -> String {
        match item.0.get(variable) {
            Some(Value::Date(DateValue::DateParts {
                date_parts,
                literal,
                ..
            })) => {
                let parts: Vec<&Vec<i16>> = date_parts.0.iter().map(|d| &d.0).collect();
                format!("{parts:?} {}", literal.as_deref().unwrap_or_default())
            }
            _ => String::from("none"),
        }
    }

    #[test]
    fn an_article_is_an_article_with_an_issue() {
        let item = item(
            "@article{doe2020, author={Doe, Jane and Roe, Richard}, title={A Study}, \
             subtitle={Of Lists}, journal={J. Stud.}, year={2020}, volume={3}, number={2}, \
             pages={1--9}, doi={10.1000/x}}",
        );
        assert_eq!(string(&item, "id"), Some("doe2020"));
        assert_eq!(string(&item, "type"), Some("article-journal"));
        assert_eq!(string(&item, "title"), Some("A study: Of lists"));
        assert_eq!(string(&item, "title-short"), Some("A study"));
        assert_eq!(string(&item, "container-title"), Some("J. Stud."));
        assert_eq!(string(&item, "volume"), Some("3"));
        assert_eq!(string(&item, "issue"), Some("2"));
        assert_eq!(string(&item, "page"), Some("1-9"));
        assert_eq!(string(&item, "DOI"), Some("10.1000/x"));
        assert_eq!(names(&item, "author"), ["Doe|Jane|||", "Roe|Richard|||"]);
        assert_eq!(date(&item, "issued"), "[[2020]] ");
    }

    #[test]
    fn entry_types_map_as_pandoc_maps_them() {
        let cases = [
            ("@book{k, title={T}}", Some("book")),
            ("@inproceedings{k, title={T}}", Some("paper-conference")),
            ("@incollection{k, title={T}}", Some("chapter")),
            ("@phdthesis{k, title={T}}", Some("thesis")),
            ("@techreport{k, title={T}}", Some("report")),
            ("@online{k, title={T}}", Some("webpage")),
            ("@electronic{k, title={T}}", Some("webpage")),
            ("@unpublished{k, title={T}}", Some("manuscript")),
            ("@unpublished{k, title={T}, eventtitle={E}}", Some("speech")),
            (
                "@article{k, title={T}, entrysubtype={newspaper}}",
                Some("article-newspaper"),
            ),
            ("@misc{k, title={T}}", None),
            ("@whatever{k, title={T}}", None),
        ];
        for (bib, expected) in cases {
            assert_eq!(string(&item(bib), "type"), expected, "{bib}");
        }
    }

    #[test]
    fn a_thesis_and_a_report_keep_their_institution_and_genre() {
        let thesis = item("@phdthesis{k, title={T}, school={MIT}, year={2001}}");
        assert_eq!(string(&thesis, "publisher"), Some("MIT"));
        assert_eq!(string(&thesis, "genre"), Some("PhD thesis"));
        let masters = item("@mastersthesis{k, title={T}, type={mathesis}}");
        assert_eq!(string(&masters, "genre"), Some("Master's thesis"));
        let report = item(
            "@techreport{k, title={T}, institution={NASA}, number={TR-7}, type={Technical Memo}}",
        );
        assert_eq!(string(&report, "publisher"), Some("NASA"));
        assert_eq!(string(&report, "number"), Some("TR-7"));
        assert_eq!(string(&report, "genre"), Some("Technical Memo"));
        let book = item("@book{k, title={T}, series={S}, number={4}, publisher={A and B}}");
        assert_eq!(string(&book, "collection-number"), Some("4"));
        assert_eq!(string(&book, "publisher"), Some("A; B"));
    }

    #[test]
    fn in_press_is_a_status_and_a_literal_date() {
        let pubstate = item("@article{k, title={T}, pubstate={inpress}, journal={J}}");
        assert_eq!(string(&pubstate, "status"), Some("in press"));
        assert!(!pubstate.0.contains_key("issued"));
        let year = item("@article{k, title={T}, year={in press}, journal={J}}");
        assert_eq!(date(&year, "issued"), "[] in press");
        assert_eq!(string(&year, "status"), Some("in press"));
        let odd = item("@article{k, title={T}, year={soon}, journal={J}}");
        assert_eq!(date(&odd, "issued"), "[] soon");
        assert_eq!(string(&odd, "status"), None);
    }

    #[test]
    fn dates_come_from_date_or_year_month_day_with_ranges_and_origdate() {
        let full = item("@misc{k, title={T}, year={2019}, month={mar}, day={4}}");
        assert_eq!(date(&full, "issued"), "[[2019, 3, 4]] ");
        let range = item(
            "@misc{k, title={T}, date={2019-03/2020-01}, origdate={1905}, urldate={2021-05-06}}",
        );
        assert_eq!(date(&range, "issued"), "[[2019, 3], [2020, 1]] ");
        assert_eq!(date(&range, "original-date"), "[[1905]] ");
        assert_eq!(date(&range, "accessed"), "[[2021, 5, 6]] ");
    }

    #[test]
    fn names_have_particles_literals_suffixes_and_spaced_initials() {
        let item = item(
            "@misc{k, title={T}, author={van der Berg, Ludwig and {Copernicus Climate Change Service} \
             and Smith, Jr., J.G. and Plato}, editor={Doe, Jane}, editortype={director}}",
        );
        assert_eq!(
            names(&item, "author"),
            [
                "Berg|Ludwig||van der|",
                "{Copernicus Climate Change Service}",
                "Smith|J. G.|||Jr.",
                "Plato||||",
            ]
        );
        assert_eq!(names(&item, "director"), ["Doe|Jane|||"]);
        assert!(names(&item, "editor").is_empty());
    }

    #[test]
    fn a_useprefix_particle_is_non_dropping() {
        let item = item(
            "@misc{k, title={T}, author={family=Berg, given=Ludwig, prefix=van der, useprefix=true}}",
        );
        assert_eq!(names(&item, "author"), ["Berg|Ludwig|van der||"]);
    }

    #[test]
    fn a_web_page_keeps_its_site_and_an_eprint_gives_a_url() {
        // BibLaTeX lists `organization` with "and"; braces keep a name whole.
        let page = item(
            "@online{k, title={T}, organization={{Hyperbole and a Half}}, url={https://x.org}}",
        );
        assert_eq!(string(&page, "publisher"), Some("Hyperbole and a Half"));
        assert_eq!(string(&page, "URL"), Some("https://x.org"));
        let arxiv = item("@misc{k, title={T}, eprint={2101.00001}, eprinttype={arXiv}}");
        assert_eq!(
            string(&arxiv, "URL"),
            Some("https://arxiv.org/abs/2101.00001")
        );
        let magazine = item(
            "@article{k, title={T}, entrysubtype={magazine}, note={N}, addendum={A}, \
             langid={german}, keywords={a, b}}",
        );
        assert_eq!(string(&magazine, "type"), Some("article-magazine"));
        assert_eq!(string(&magazine, "note"), Some("N. A"));
        assert_eq!(string(&magazine, "language"), Some("de-DE"));
        assert_eq!(string(&magazine, "keyword"), Some("a, b"));
    }

    #[test]
    fn a_chapter_has_its_book_as_container_once() {
        let chapter = item(
            "@incollection{k, title={T}, booktitle={Bioinformatics}, bookauthor={Doe, Jane}, \
             publisher={P}, address={Berlin}, chapter={3}, edition={2}}",
        );
        assert_eq!(string(&chapter, "container-title"), Some("Bioinformatics"));
        assert_eq!(string(&chapter, "publisher-place"), Some("Berlin"));
        assert_eq!(string(&chapter, "chapter-number"), Some("3"));
        assert_eq!(string(&chapter, "edition"), Some("2"));
        assert_eq!(names(&chapter, "container-author"), ["Doe|Jane|||"]);
        let volume = item("@book{k, title={Vol. 2}, maintitle={Collected Works}, volume={2}}");
        assert_eq!(string(&volume, "title"), Some("Collected works"));
        assert_eq!(string(&volume, "volume-title"), Some("Vol. 2"));
    }

    #[test]
    fn a_chapter_of_a_multi_volume_work_is_in_its_main_title() {
        let chapter = item(
            "@incollection{k, title={The Chapter}, booktitle={The Book Title}, \
             maintitle={Collected Works}, mainsubtitle={A Critical Edition}, volume={3}}",
        );
        assert_eq!(
            string(&chapter, "container-title"),
            Some("Collected works: A critical edition")
        );
        assert_eq!(string(&chapter, "volume-title"), Some("The book title"));
        assert_eq!(string(&chapter, "volume"), Some("3"));
        let volume = item(
            "@book{k, title={The Volume}, maintitle={Collected Works}, \
             mainsubtitle={A Critical Edition}, volume={3}}",
        );
        assert_eq!(
            string(&volume, "title"),
            Some("Collected works: A critical edition")
        );
        assert_eq!(string(&volume, "volume-title"), Some("The volume"));
        assert_eq!(string(&volume, "container-title"), None);
    }

    #[test]
    fn latex_conventions_become_text() {
        let item = item(
            "@misc{k, title={Caf\\'e {NASA} --- a tale, 1--2 \\& more~here}, author={M\\\"uller, Hans}}",
        );
        assert_eq!(
            string(&item, "title"),
            Some("Café NASA \u{2014} a tale, 1\u{2013}2 & more\u{a0}here")
        );
        assert_eq!(names(&item, "author"), ["Müller|Hans|||"]);
    }

    #[test]
    fn english_titles_go_to_sentence_case_and_braces_protect() {
        let english = item(
            "@incollection{k, title={The Alot Is Better Than {You} at Everything}, \
             booktitle={Collected {NASA} Posts}, series={Great Series}}",
        );
        assert_eq!(
            string(&english, "title"),
            Some("The alot is better than You at everything")
        );
        assert_eq!(
            string(&english, "container-title"),
            Some("Collected NASA posts")
        );
        assert_eq!(string(&english, "collection-title"), Some("Great series"));
        let german = item("@book{k, title={Die Welt Als Wille}, langid={german}}");
        assert_eq!(string(&german, "title"), Some("Die Welt Als Wille"));
        let bibliography =
            biblatex::Bibliography::parse("@book{k, title={Die Welt Als Wille}}").unwrap();
        let in_german_document = item_of(bibliography.iter().next().unwrap(), false);
        assert_eq!(
            string(&in_german_document, "title"),
            Some("Die Welt Als Wille")
        );
        let article = item("@article{k, title={T}, journal={Journal of Studies}}");
        assert_eq!(
            string(&article, "container-title"),
            Some("Journal of Studies")
        );
    }

    #[test]
    fn list_fields_join_their_parts_with_a_semicolon() {
        let book = item(
            "@book{k, title={T}, location={Durham and London}, origlocation={A and B}, \
             publisher={P1 and {{P2 and P3}}}}",
        );
        assert_eq!(string(&book, "publisher-place"), Some("Durham; London"));
        assert_eq!(string(&book, "original-publisher-place"), Some("A; B"));
        assert_eq!(string(&book, "publisher"), Some("P1; P2 and P3"));
        let addressed = item("@book{k, title={T}, address={A1 and A2}, location={L}}");
        assert_eq!(string(&addressed, "publisher-place"), Some("A1; A2"));
    }

    #[test]
    fn a_numbered_series_is_an_ordinal_series() {
        let cases = [
            ("1", "1st series"),
            ("2", "2nd series"),
            ("3", "3rd series"),
            ("4", "4th series"),
            ("11", "11th series"),
            ("12", "12th series"),
            ("13", "13th series"),
            ("21", "21st series"),
            ("Quarterly", "Quarterly"),
        ];
        for (series, expected) in cases {
            let article = item(&format!(
                "@article{{k, title={{T}}, journaltitle={{J}}, series={{{series}}}}}"
            ));
            assert_eq!(string(&article, "collection-title"), Some(expected));
        }
    }

    #[test]
    fn a_short_title_is_the_part_before_the_colon() {
        let colon = item("@book{k, title={Molecular Cloning: A Laboratory Manual}}");
        assert_eq!(
            string(&colon, "title"),
            Some("Molecular cloning: A laboratory manual")
        );
        assert_eq!(string(&colon, "title-short"), Some("Molecular cloning"));
        let plain = item("@book{k, title={No Colon Here}}");
        assert_eq!(string(&plain, "title-short"), None);
    }

    #[test]
    fn a_periodical_is_an_issue_of_the_journal_its_title_names() {
        let issue = item(
            "@periodical{k, title={Computers and Graphics}, \
             issuetitle={Semantic {3D} Media}, issuesubtitle={A Survey}}",
        );
        assert_eq!(string(&issue, "type"), Some("article-journal"));
        assert_eq!(string(&issue, "title"), Some("Semantic 3D media: A survey"));
        assert_eq!(
            string(&issue, "container-title"),
            Some("Computers and Graphics")
        );
        assert_eq!(
            string(&issue, "title-short"),
            Some("Computers and graphics")
        );
        let journal = item("@periodical{k, title={Computers and Graphics}, shorttitle={CaG}}");
        assert_eq!(string(&journal, "title"), Some("Computers and graphics"));
        assert_eq!(
            string(&journal, "container-title"),
            Some("Computers and Graphics")
        );
        assert_eq!(string(&journal, "container-title-short"), Some("CaG"));
    }

    #[test]
    fn the_entry_options_set_useprefix_for_every_name() {
        let on =
            item("@misc{k, title={T}, author={van Dijk, Dirk}, options={doi=false, useprefix}}");
        assert_eq!(names(&on, "author"), ["Dijk|Dirk|van||"]);
        let off = item("@misc{k, title={T}, author={van Dijk, Dirk}, options={useprefix=false}}");
        assert_eq!(names(&off, "author"), ["Dijk|Dirk||van|"]);
    }

    #[test]
    fn the_entry_options_url_false_and_doi_false_leave_the_field_out() {
        let fields = "title={T}, url={http://u}, doi={1/2}";
        let doi_off = item(&format!("@article{{k, {fields}, options={{doi=false}}}}"));
        assert_eq!(string(&doi_off, "DOI"), None);
        assert_eq!(string(&doi_off, "URL"), Some("http://u"));
        let url_off = item(&format!(
            "@article{{k, {fields}, options={{ URL = False }}}}"
        ));
        assert_eq!(string(&url_off, "URL"), None);
        assert_eq!(string(&url_off, "DOI"), Some("1/2"));
        let plain = item(&format!(
            "@article{{k, {fields}, options={{doi, url=true}}}}"
        ));
        assert_eq!(string(&plain, "URL"), Some("http://u"));
        assert_eq!(string(&plain, "DOI"), Some("1/2"));
        let online = item(&format!("@online{{k, {fields}, options={{url=false}}}}"));
        assert_eq!(string(&online, "URL"), Some("http://u"));
    }

    #[test]
    fn apostrophes_and_quotes_turn_typographic_but_identifiers_stay() {
        let item = item(
            "@misc{k, title={Shin'ya's ``Quoted'' Words}, author={Shin'ya, Ono}, \
             url={https://x.org/~me/it's}, doi={10.1/a'b}}",
        );
        assert_eq!(
            string(&item, "title"),
            Some("Shin\u{2019}ya\u{2019}s \u{201c}quoted\u{201d} words")
        );
        assert_eq!(names(&item, "author"), ["Shin\u{2019}ya|Ono|||"]);
        assert_eq!(string(&item, "URL"), Some("https://x.org/~me/it's"));
        assert_eq!(string(&item, "DOI"), Some("10.1/a'b"));
    }

    #[test]
    fn sentence_case_lowers_only_words_with_a_single_leading_capital() {
        let cases = [
            (
                "DNA Repair and the NASA Archive",
                "DNA repair and the NASA archive",
            ),
            ("COVID-19 and the Lab", "COVID-19 and the lab"),
            ("World War II Records", "World war II records"),
            (
                "iPhone Apps and McDonald Stories",
                "iPhone apps and McDonald stories",
            ),
            ("eBay Markets", "eBay markets"),
            ("Why Cite? A Short Answer", "Why cite? A short answer"),
            (
                "Molecular cloning: a laboratory manual",
                "Molecular cloning: A laboratory manual",
            ),
            ("molecular cloning", "Molecular cloning"),
            (
                "Long-Term Effects of Self-Citation",
                "Long-term effects of self-citation",
            ),
            ("Walking in {London} Today", "Walking in London today"),
            ("A Study of I and You", "A study of i and you"),
            (
                "Ratio a:b and http://x.org: The End",
                "Ratio a:b and http://x.org: The end",
            ),
            (
                "Tags <mml:Mi>A</mml:Mi> Here",
                "Tags <mml:mi>a</mml:mi> here",
            ),
        ];
        for (title, expected) in cases {
            let entry = item(&format!("@book{{k, title={{{title}}}}}"));
            assert_eq!(string(&entry, "title"), Some(expected), "{title}");
        }
    }

    #[test]
    fn localisation_keys_print_their_english_text() {
        let patent = item("@patent{p, title={T}, location={Near countryfr}, type={patenteu}}");
        assert_eq!(string(&patent, "jurisdiction"), Some("Near France"));
        assert_eq!(string(&patent, "genre"), Some("European patent"));
        let book = item(
            r"@book{b, title={The Title: \bibstring{newseries}},
              note={In \bibstring{countryuk}}, location={countryfr}}",
        );
        assert_eq!(string(&book, "title"), Some("The title: New series"));
        assert_eq!(string(&book, "note"), Some("In United Kingdom"));
        assert_eq!(string(&book, "publisher-place"), Some("countryfr"));
    }
}
