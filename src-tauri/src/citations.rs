//! Rendering of citations and the reference list with hayagriva.
//!
//! The style and the locale come from the document header: `csl:` names one
//! of the bundled Citation Style Language styles, `lang:` picks the locale.
//! Custom style files are not read; a style name is all the header can give.

use hayagriva::archive::ArchivedStyle;
use hayagriva::citationberg::{IndependentStyle, LocaleCode, Style};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
