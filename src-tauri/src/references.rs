//! Reading the references a document names in its header.
//!
//! The editor offers them after `@` and shows one on hover, so what leaves
//! this module is a short summary per entry, the same for every source format.

use hayagriva::citationberg::json::{FixedDateRange, Item, NameValue, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// One reference, as the editor lists it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ReferenceSummary {
    /// The citation key, the part after `@`.
    pub key: String,
    /// Family name first ("Doe, Jane"); an institution has only its name.
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub title: String,
}

/// The entries a source gave, and a warning for each entry it had to skip.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct References {
    pub entries: Vec<ReferenceSummary>,
    pub warnings: Vec<String>,
}

/// The entries of a `.bib` file, in file order.
///
/// hayagriva reads a file as a whole and refuses all of it over one entry it
/// cannot take. Real files have such entries (Crossref exports `month = July`
/// and repeats keys, authors write `year = {in press}`), and an editor is
/// better served by the rest of the library. So the file is then read entry
/// by entry: what converts is kept, and each entry skipped gives a warning
/// with its line.
pub fn references_from_biblatex(text: &str) -> References {
    if let Ok(library) = hayagriva::io::from_biblatex_str(text) {
        let entries = library.iter().map(summarize).collect();
        return References {
            entries,
            warnings: Vec::new(),
        };
    }
    let mut found = References::default();
    let mut definitions = String::new();
    let mut keys = HashSet::new();
    for (line, chunk) in entry_chunks(text) {
        let Some(bibliography) = parse_chunk(chunk, line, &mut definitions, &mut found.warnings)
        else {
            continue;
        };
        // An abbreviation defined here serves the entries after it.
        if chunk[1..]
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("string")
        {
            definitions.push_str(chunk);
            definitions.push('\n');
        }
        for raw in bibliography.iter() {
            if !keys.insert(raw.key.clone()) {
                found.warnings.push(format!(
                    "line {line}: key {} is used again, this entry is skipped",
                    raw.key
                ));
                continue;
            }
            match hayagriva::Entry::try_from(raw) {
                Ok(entry) => found.entries.push(summarize(&entry)),
                Err(e) => found.warnings.push(format!(
                    "line {}: entry {} skipped, {}",
                    line + line_of(e.span.start, chunk) - 1,
                    raw.key,
                    e.kind
                )),
            }
        }
    }
    found
}

/// The file cut at every line that starts an entry (`@` at line start), each
/// chunk with its first line number. Text before the first entry is ignored,
/// as BibTeX does.
fn entry_chunks(text: &str) -> Vec<(usize, &str)> {
    let mut chunks: Vec<(usize, usize)> = Vec::new();
    let mut offset = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if line.trim_start().starts_with('@') {
            chunks.push((index + 1, offset));
        }
        offset += line.len();
    }
    let ends = chunks
        .iter()
        .skip(1)
        .map(|(_, start)| *start)
        .chain([text.len()]);
    chunks
        .iter()
        .zip(ends)
        .map(|(&(line, start), end)| (line, &text[start..end]))
        .collect()
}

/// How many undefined abbreviations one entry may use before it is skipped.
const UNDEFINED_ABBREVIATIONS_LIMIT: usize = 16;

/// One chunk parsed with the abbreviations defined so far. An abbreviation
/// the file never defines is taken as its own text and defined for the
/// entries after it, with a warning. Any other problem skips the chunk.
fn parse_chunk(
    chunk: &str,
    line: usize,
    definitions: &mut String,
    warnings: &mut Vec<String>,
) -> Option<biblatex::Bibliography> {
    for _ in 0..UNDEFINED_ABBREVIATIONS_LIMIT {
        // Definitions after the entry, so its lines keep their numbers.
        let error = match biblatex::Bibliography::parse(&format!("{chunk}\n{definitions}")) {
            Ok(bibliography) => return Some(bibliography),
            Err(error) => error,
        };
        let at = line + line_of(error.span.start.min(chunk.len()), chunk) - 1;
        match error.kind {
            biblatex::ParseErrorKind::UnknownAbbreviation(name) => {
                warnings.push(format!(
                    "line {at}: abbreviation {name} is not defined, taken as text"
                ));
                definitions.push_str(&format!("@string{{{name} = {{{name}}}}}\n"));
            }
            kind => {
                warnings.push(format!("line {at}: entry skipped, {kind}"));
                return None;
            }
        }
    }
    None
}

fn summarize(entry: &hayagriva::Entry) -> ReferenceSummary {
    ReferenceSummary {
        key: entry.key().to_string(),
        authors: entry
            .authors()
            .unwrap_or_default()
            .iter()
            .map(|person| person.name_first(false, false))
            .collect(),
        year: entry.date().map(|date| date.year),
        title: entry.title().map(ToString::to_string).unwrap_or_default(),
    }
}

/// The entries of CSL-JSON text, the list of items a reference manager exports.
pub fn references_from_csl_json(text: &str) -> Result<References, String> {
    let items: Vec<Item> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(summarize_items(&items))
}

/// The entries of CSL-YAML text: the `references:` list of a pandoc
/// bibliography file or of a document's header, or a bare list of items.
pub fn references_from_csl_yaml(text: &str) -> Result<References, String> {
    use yaml_serde::Value as Yaml;
    let document: Yaml = yaml_serde::from_str(text).map_err(|e| e.to_string())?;
    let list = match document {
        Yaml::Mapping(mut map) => map.remove("references").unwrap_or(Yaml::Null),
        other => other,
    };
    let items: Vec<Item> = match list {
        Yaml::Null => Vec::new(),
        list => yaml_serde::from_value(list).map_err(|e| e.to_string())?,
    };
    Ok(summarize_items(&items))
}

/// The files a header names under `bibliography:`, one path or a list of
/// paths as pandoc allows, in header order. Paths are returned as written;
/// where they may point is decided when they are read.
pub fn bibliography_paths(front_matter: &str) -> Result<Vec<String>, String> {
    use yaml_serde::Value as Yaml;
    let document: Yaml = yaml_serde::from_str(front_matter).map_err(|e| e.to_string())?;
    let Yaml::Mapping(map) = document else {
        return Ok(Vec::new());
    };
    Ok(match map.get("bibliography") {
        Some(Yaml::String(path)) => vec![path.clone()],
        Some(Yaml::Sequence(list)) => list
            .iter()
            .filter_map(|path| path.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    })
}

/// The reference file formats, by extension.
const REFERENCE_EXTS: &[&str] = &["bib", "json", "yaml", "yml"];

/// Where a `bibliography:` path may point: a file in the document's folder
/// or below, in one of the reference formats. The document chooses the path,
/// and a document someone else wrote must not read arbitrary files or fetch
/// anything, so an absolute path, `..`, and a web address are refused.
pub fn bibliography_file(document_dir: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    let escapes = relative.is_absolute()
        || relative
            .components()
            .any(|part| part == Component::ParentDir);
    if escapes || path.contains("://") {
        return Err(format!(
            "{path}: only a file in the document's folder is read"
        ));
    }
    let ext = relative
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if !ext.is_some_and(|e| REFERENCE_EXTS.contains(&e.as_str())) {
        return Err(format!("{path}: not a .bib, .json, .yaml or .yml file"));
    }
    Ok(document_dir.join(relative))
}

/// The largest reference file that is read. A library export of thousands of
/// entries is a few megabytes; the whole file is held in memory while read.
const BIBLIOGRAPHY_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// The entries of one `bibliography:` file of the document in `document_dir`.
/// `allow_network` is the user's setting for network paths, see validate_path.
pub fn read_bibliography(
    document_dir: &Path,
    path: &str,
    allow_network: bool,
) -> Result<References, String> {
    let file = bibliography_file(document_dir, path)?;
    let file_str = file.to_string_lossy();
    crate::validate_path(&file_str, allow_network)?;
    // A link inside the folder can still lead out of it; the resolved path
    // has to stay below the resolved folder.
    let resolved = fs::canonicalize(&file).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{path}: not found"),
        _ => format!("{path}: {e}"),
    })?;
    let root = fs::canonicalize(document_dir).map_err(|e| e.to_string())?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "{path}: only a file in the document's folder is read"
        ));
    }
    let bytes = crate::read_capped(&file_str, BIBLIOGRAPHY_MAX_BYTES)
        .map_err(|e| format!("{path}: {e}"))?
        .ok_or_else(|| format!("{path}: larger than {} MB", BIBLIOGRAPHY_MAX_BYTES >> 20))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{path}: not UTF-8 text"))?;
    match file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
    {
        Some(ext) if ext == "bib" => Ok(references_from_biblatex(&text)),
        Some(ext) if ext == "json" => references_from_csl_json(&text),
        _ => references_from_csl_yaml(&text),
    }
    .map_err(|e| format!("{path}: {e}"))
}

/// One place the header named and how many entries it gave; a source that
/// could not be read gives none, and its warning says why.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Source {
    /// `references:` for the inline list, else the path as written.
    pub name: String,
    pub entries: usize,
}

/// What the editor gets for a document: every entry, every warning, and the
/// sources in header order.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Loaded {
    pub entries: Vec<ReferenceSummary>,
    pub warnings: Vec<String>,
    pub sources: Vec<Source>,
}

impl Loaded {
    fn add(&mut self, name: &str, result: Result<References, String>) {
        let mut entries = 0;
        match result {
            Ok(found) => {
                entries = found.entries.len();
                self.entries.extend(found.entries);
                self.warnings.extend(found.warnings);
            }
            Err(problem) => self.warnings.push(problem),
        }
        self.sources.push(Source {
            name: name.to_string(),
            entries,
        });
    }
}

/// Everything the header of the document at `document_path` names: the
/// inline `references:` list first, then each `bibliography:` file in header
/// order. A file that cannot be read becomes a warning, so the editor keeps
/// what the other sources gave. An unsaved document has no folder for the
/// files to be relative to, so they wait until it is saved.
pub fn load(document_path: Option<&str>, front_matter: &str, allow_network: bool) -> Loaded {
    let mut loaded = Loaded::default();
    let inline = match references_from_csl_yaml(front_matter) {
        Ok(inline) => inline,
        Err(problem) => {
            loaded.warnings.push(format!("header: {problem}"));
            return loaded;
        }
    };
    if front_matter
        .lines()
        .any(|line| line.starts_with("references:"))
    {
        loaded.add("references:", Ok(inline));
    }
    let dir = document_path.map(Path::new).and_then(Path::parent);
    for path in bibliography_paths(front_matter).unwrap_or_default() {
        let result = match dir {
            Some(dir) => read_bibliography(dir, &path, allow_network),
            None => Err(format!(
                "{path}: save the document first, the path is relative to it"
            )),
        };
        loaded.add(&path, result);
    }
    loaded
}

fn summarize_items(items: &[Item]) -> References {
    let mut references = References::default();
    for (index, item) in items.iter().enumerate() {
        // Without an `id` there is nothing to write after `@`.
        match item.id() {
            Some(id) => references.entries.push(summarize_item(&id, item)),
            None => references
                .warnings
                .push(format!("entry {} skipped, it has no id", index + 1)),
        }
    }
    references
}

fn summarize_item(id: &str, item: &Item) -> ReferenceSummary {
    let authors = match item.0.get("author") {
        Some(Value::Names(names)) => names.iter().map(name_first).collect(),
        _ => Vec::new(),
    };
    let title = item.0.get("title").and_then(Value::to_str);
    ReferenceSummary {
        key: id.to_string(),
        authors,
        year: item.0.get("issued").and_then(year_of),
        title: title.unwrap_or_default().into_owned(),
    }
}

/// Family name first, as for a `.bib` entry.
fn name_first(name: &NameValue) -> String {
    match name {
        NameValue::Literal(name) => name.literal.clone(),
        NameValue::Item(name) => {
            let particle = name.non_dropping_particle.as_deref();
            let family = particle.map_or(name.family.clone(), |p| format!("{p} {}", name.family));
            name.given
                .as_ref()
                .map_or(family.clone(), |given| format!("{family}, {given}"))
        }
    }
}

/// CSL allows a date as a bare year, as text ("2019-07") or as date parts.
fn year_of(issued: &Value) -> Option<i32> {
    let range = match issued {
        Value::Number(year) => return i32::try_from(*year).ok(),
        Value::String(text) => text.parse::<FixedDateRange>().ok()?,
        Value::Date(date) => FixedDateRange::try_from(date.clone()).ok()?,
        Value::Names(_) => return None,
    };
    Some(range.start.year.into())
}

/// The parser reports a byte position; a line number is what finds the spot.
fn line_of(position: usize, text: &str) -> usize {
    let before = &text.as_bytes()[..position.min(text.len())];
    before.iter().filter(|byte| **byte == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"@article{doe2020,
  author = {Doe, Jane and van der Roe, Richard},
  title = {A study of {ERA5} d{\"a}ta},
  year = {2020},
}

@dataset{ERA5LandHourlyData2019,
  author = {{Copernicus Climate Change Service}},
  title = {ERA5-Land hourly data from 1950 to present},
  date = {2019-07-12},
}

@misc{bare, title = {No author, no date}}
"#;

    #[test]
    fn bib_entries_come_in_file_order_with_key_year_and_title() {
        let References { entries, warnings } = references_from_biblatex(SAMPLE);
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["doe2020", "ERA5LandHourlyData2019", "bare"]);
        // From `year` or `date`, and missing where the entry has neither.
        let years: Vec<Option<i32>> = entries.iter().map(|e| e.year).collect();
        assert_eq!(years, [Some(2020), Some(2019), None]);
        // Braces and LaTeX accents are resolved.
        assert_eq!(entries[0].title, "A study of ERA5 däta");
        assert!(warnings.is_empty());
    }

    #[test]
    fn bib_authors_are_family_name_first_and_an_institution_stays_whole() {
        let entries = references_from_biblatex(SAMPLE).entries;
        assert_eq!(entries[0].authors, ["Doe, Jane", "van der Roe, Richard"]);
        assert_eq!(entries[1].authors, ["Copernicus Climate Change Service"]);
        assert!(entries[2].authors.is_empty());
    }

    #[test]
    fn an_entry_that_cannot_be_read_is_skipped_with_a_warning() {
        let odd = "@article{odd,\n  title = {T},\n  year = {in press},\n}\n";
        let found = references_from_biblatex(&format!("{SAMPLE}{odd}"));
        assert_eq!(found.entries.len(), 3);
        assert_eq!(
            found.warnings,
            ["line 16: entry odd skipped, wrong number of digits"]
        );
    }

    #[test]
    fn csl_json_items_give_the_same_summary_as_bib_entries() {
        let json = r#"[
          {"id": "doe2020", "title": "A study", "issued": {"date-parts": [[2020, 3]]},
           "author": [{"family": "Doe", "given": "Jane"},
                      {"family": "Roe", "given": "Richard", "non-dropping-particle": "van der"}]},
          {"id": "era5", "title": "ERA5-Land", "issued": 2019,
           "author": [{"literal": "Copernicus Climate Change Service"}]},
          {"id": "text", "issued": "2018-07"},
          {"title": "No id"}
        ]"#;
        let References { entries, warnings } = references_from_csl_json(json).unwrap();
        let years: Vec<Option<i32>> = entries.iter().map(|e| e.year).collect();
        assert_eq!(years, [Some(2020), Some(2019), Some(2018)]);
        assert_eq!(entries[0].authors, ["Doe, Jane", "van der Roe, Richard"]);
        assert_eq!(entries[1].authors, ["Copernicus Climate Change Service"]);
        assert_eq!(entries[1].title, "ERA5-Land");
        assert_eq!(warnings, ["entry 4 skipped, it has no id"]);
        let broken = references_from_csl_json("[{").unwrap_err();
        assert!(broken.contains("line 1"));
    }

    #[test]
    fn csl_yaml_gives_the_same_items_from_a_references_list_or_a_bare_list() {
        let header = "title: My paper\ncsl: apa\nreferences:\n- id: era5\n  type: dataset\n  title: ERA5-Land\n  author:\n  - literal: Copernicus Climate Change Service\n  issued: 2019\n";
        let found = references_from_csl_yaml(header).unwrap();
        assert_eq!(found.entries.len(), 1);
        assert_eq!(found.entries[0].key, "era5");
        assert_eq!(
            found.entries[0].authors,
            ["Copernicus Climate Change Service"]
        );
        assert_eq!(found.entries[0].year, Some(2019));
        let bare = "- id: doe\n  title: A study\n  issued:\n    date-parts: [[2020, 3]]\n";
        let found = references_from_csl_yaml(bare).unwrap();
        assert_eq!(
            (found.entries[0].year, found.entries[0].title.as_str()),
            (Some(2020), "A study")
        );
        assert_eq!(
            references_from_csl_yaml("title: No references\n"),
            Ok(References::default())
        );
        assert_eq!(references_from_csl_yaml(""), Ok(References::default()));
        assert!(references_from_csl_yaml("- id: [\n")
            .unwrap_err()
            .contains("line"));
    }

    #[test]
    fn bibliography_paths_come_from_one_value_or_a_list() {
        let one = "title: T\nbibliography: refs.bib\n";
        assert_eq!(bibliography_paths(one), Ok(vec!["refs.bib".to_string()]));
        let many = "bibliography:\n- refs.bib\n- \"My Library.json\"\n";
        assert_eq!(
            bibliography_paths(many),
            Ok(vec!["refs.bib".to_string(), "My Library.json".to_string()])
        );
        assert_eq!(bibliography_paths("title: T\n"), Ok(vec![]));
        assert_eq!(bibliography_paths(""), Ok(vec![]));
        assert_eq!(bibliography_paths("- not a header\n"), Ok(vec![]));
        assert!(bibliography_paths("bibliography: [\n").is_err());
    }

    #[test]
    fn a_bibliography_file_stays_in_the_document_folder_with_a_known_format() {
        let dir = Path::new("/docs/paper");
        let file = |path: &str| bibliography_file(dir, path);
        assert_eq!(file("refs.bib"), Ok(PathBuf::from("/docs/paper/refs.bib")));
        assert_eq!(
            file("lib/Zotero.JSON"),
            Ok(PathBuf::from("/docs/paper/lib/Zotero.JSON"))
        );
        let outside = "only a file in the document's folder is read";
        assert!(file("/etc/passwd.bib").unwrap_err().ends_with(outside));
        assert!(file("../shared/refs.bib").unwrap_err().ends_with(outside));
        assert!(file("https://example.org/refs.bib")
            .unwrap_err()
            .ends_with(outside));
        assert!(file("notes.txt")
            .unwrap_err()
            .ends_with("not a .bib, .json, .yaml or .yml file"));
        assert!(file("refs").is_err());
    }

    #[test]
    fn a_bibliography_file_is_read_by_its_format_from_inside_the_folder() {
        let dir = std::env::temp_dir().join(format!("monoleaf_refs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(dir.join("lib/refs.bib"), SAMPLE).unwrap();
        fs::write(
            dir.join("refs.json"),
            r#"[{"id": "j", "title": "From JSON"}]"#,
        )
        .unwrap();
        fs::write(
            dir.join("refs.yml"),
            "references:\n- id: y\n  title: From YAML\n",
        )
        .unwrap();
        let read = |path: &str| read_bibliography(&dir, path, false);
        assert_eq!(read("lib/refs.bib").unwrap().entries.len(), 3);
        assert_eq!(read("refs.json").unwrap().entries[0].title, "From JSON");
        assert_eq!(read("refs.yml").unwrap().entries[0].key, "y");
        assert_eq!(read("missing.bib"), Err("missing.bib: not found".into()));
        #[cfg(unix)]
        {
            // A link that leads out of the folder is refused like `..`.
            let outside = dir.with_extension("outside.bib");
            fs::write(&outside, SAMPLE).unwrap();
            std::os::unix::fs::symlink(&outside, dir.join("link.bib")).unwrap();
            assert!(read("link.bib").unwrap_err().ends_with("folder is read"));
            let _ = fs::remove_file(outside);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_joins_the_inline_list_and_the_files_and_warns_per_problem() {
        let dir = std::env::temp_dir().join(format!("monoleaf_load-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("refs.bib"), SAMPLE).unwrap();
        let doc = dir.join("paper.md");
        let header = "bibliography:\n- refs.bib\n- missing.bib\nreferences:\n- id: inline\n  title: Inline\n";
        let found = load(doc.to_str(), header, false);
        let keys: Vec<&str> = found.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            ["inline", "doe2020", "ERA5LandHourlyData2019", "bare"]
        );
        assert_eq!(found.warnings, ["missing.bib: not found"]);
        let sources: Vec<(&str, usize)> = found
            .sources
            .iter()
            .map(|s| (s.name.as_str(), s.entries))
            .collect();
        assert_eq!(
            sources,
            [("references:", 1), ("refs.bib", 3), ("missing.bib", 0)]
        );
        let unsaved = load(None, header, false);
        assert_eq!(unsaved.entries.len(), 1);
        assert!(unsaved.warnings[0].starts_with("refs.bib: save the document first"));
        assert!(load(None, "references: [\n", false).warnings[0].starts_with("header: "));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_undefined_abbreviation_is_taken_as_its_text_with_a_warning() {
        let crossref =
            "@article{a, title={T}, year={2021}, month=July}\n@misc{b, title={U}, journal=jphys}\n";
        let found = references_from_biblatex(crossref);
        assert_eq!(found.entries.len(), 2);
        assert_eq!(
            found.warnings,
            [
                "line 1: abbreviation July is not defined, taken as text",
                "line 2: abbreviation jphys is not defined, taken as text",
            ]
        );
    }

    #[test]
    fn a_broken_or_repeated_entry_is_skipped_with_its_line_and_the_rest_stays() {
        let unclosed = "@article{a,\n  title = {Unclosed,\n}\n@misc{b, title = {B}}\n";
        let found = references_from_biblatex(unclosed);
        assert_eq!(found.entries.len(), 1);
        assert_eq!(found.warnings, ["line 4: entry skipped, expected comma"]);
        let twice = "@misc{a, title = {A}}\n@misc{a, title = {B}}\n@misc{c, title = {C}}\n";
        let found = references_from_biblatex(twice);
        assert_eq!(found.entries.len(), 2);
        assert_eq!(
            found.warnings,
            ["line 2: key a is used again, this entry is skipped"]
        );
        assert_eq!(references_from_biblatex(""), References::default());
        assert_eq!(
            references_from_biblatex("no entries here\n"),
            References::default()
        );
    }
}
