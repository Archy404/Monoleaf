//! Reading the references a document names in its header.
//!
//! The editor offers them after `@` and shows one on hover, so what leaves
//! this module is a short summary per entry, the same for every source format.

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
