//! Reading the references a document names in its header.
//!
//! The editor offers them after `@` and shows one on hover, so what leaves
//! this module is a short summary per entry, the same for every source format.

use hayagriva::citationberg::json::{FixedDateRange, Item, NameValue, Value};
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
/// cannot interpret (`year = {in press}`). An editor is better served by the
/// rest of the library, so in that case the file is read again entry by
/// entry: what converts is kept, and each skipped entry gives a warning with
/// its line. A file that does not parse at all gives only that problem.
pub fn references_from_biblatex(text: &str) -> Result<References, String> {
    if let Ok(library) = hayagriva::io::from_biblatex_str(text) {
        let entries = library.iter().map(summarize).collect();
        return Ok(References {
            entries,
            warnings: Vec::new(),
        });
    }
    let bibliography = biblatex::Bibliography::parse(text)
        .map_err(|e| format!("line {}: {}", line_of(e.span.start, text), e.kind))?;
    let mut references = References::default();
    for raw in bibliography.iter() {
        match hayagriva::Entry::try_from(raw) {
            Ok(entry) => references.entries.push(summarize(&entry)),
            Err(e) => references.warnings.push(format!(
                "line {}: entry {} skipped, {}",
                line_of(e.span.start, text),
                raw.key,
                e.kind
            )),
        }
    }
    Ok(references)
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
        Some(ext) if ext == "bib" => references_from_biblatex(&text),
        Some(ext) if ext == "json" => references_from_csl_json(&text),
        _ => references_from_csl_yaml(&text),
    }
    .map_err(|e| format!("{path}: {e}"))
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
        let References { entries, warnings } = references_from_biblatex(SAMPLE).unwrap();
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
        let entries = references_from_biblatex(SAMPLE).unwrap().entries;
        assert_eq!(entries[0].authors, ["Doe, Jane", "van der Roe, Richard"]);
        assert_eq!(entries[1].authors, ["Copernicus Climate Change Service"]);
        assert!(entries[2].authors.is_empty());
    }

    #[test]
    fn an_entry_that_cannot_be_read_is_skipped_with_a_warning() {
        let odd = "@article{odd,\n  title = {T},\n  year = {in press},\n}\n";
        let found = references_from_biblatex(&format!("{SAMPLE}{odd}")).unwrap();
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
    fn a_bib_that_does_not_parse_reports_the_line_instead_of_entries() {
        let unclosed = "@article{a,\n  title = {Unclosed,\n}\n";
        assert_eq!(
            references_from_biblatex(unclosed),
            Err("line 4: expected comma".into())
        );
        let twice = "@misc{a, title = {A}}\n@misc{a, title = {B}}\n";
        assert_eq!(
            references_from_biblatex(twice),
            Err("line 2: duplicate key \"a\"".into())
        );
        assert_eq!(references_from_biblatex(""), Ok(References::default()));
    }
}
