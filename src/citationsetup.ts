/**
 * The references dialog's side of the citation features: what the dialog
 * shows for a header, the header lines it writes back, and the example a
 * writer can start from. Pure text work; main.ts wires the dialog.
 */

import type { HeaderValue } from "./meta";
import { findCitations } from "./citationsyntax";

/** What the dialog shows and takes. */
export interface CitationSetup {
  /** A bundled style's name, `csl:`. */
  style: string;
  /** A locale code, `lang:`; "" leaves the style's own. */
  lang: string;
  /** Files beside the document, comma-separated, `bibliography:`. */
  bibliography: string;
  /** Add the example works as an inline `references:` list. */
  inlineReferences: boolean;
  /** `reference-section-title:`; "" writes none. */
  sectionTitle: string;
  suppressBibliography: boolean;
  refsegment: boolean;
  /** Insert the template text, and the example file beside a saved document. */
  template: boolean;
}

/** pandoc's default style, the dialog's too. */
export const DEFAULT_STYLE = "chicago-author-date";

/** The example file the template cites; written beside the document. */
export const EXAMPLE_FILE = "example.bib";

export const DEFAULT_SETUP: CitationSetup = {
  style: DEFAULT_STYLE,
  lang: "",
  bibliography: "",
  inlineReferences: false,
  sectionTitle: "",
  suppressBibliography: false,
  refsegment: false,
  template: false,
};

/** The four works the template cites: an article, a book, a thesis, a web
 * page, written as a .bib file. */
export const EXAMPLE_BIB = `@article{ipsum2020,
  author  = {Ipsum, Lorem and Dolor, Sit},
  title   = {Consectetur Adipiscing: A Study of Placeholder Text},
  journal = {Journal of Examples},
  year    = {2020},
  volume  = {3},
  number  = {2},
  pages   = {1--9},
  doi     = {10.1000/example.2020.3},
}
@book{amet2019,
  author    = {Amet, Consectetur},
  title     = {The Book of Elit},
  publisher = {Example Press},
  address   = {Berlin},
  year      = {2019},
}
@phdthesis{tempor2018,
  author = {Tempor, Incididunt},
  title  = {Labore et Dolore at Scale},
  school = {Example University},
  year   = {2018},
}
@online{magna2021,
  author       = {Magna, Aliqua},
  title        = {Ut Enim ad Minim Veniam},
  organization = {{Example Site}},
  year         = {2021},
  url          = {https://example.org/ut-enim},
}
`;

/** The same four works as the lines of an inline `references:` list. */
export const EXAMPLE_REFERENCES: readonly string[] = [
  "- id: ipsum2020",
  "  type: article-journal",
  "  title: 'Consectetur adipiscing: A study of placeholder text'",
  "  author:",
  "  - family: Ipsum",
  "    given: Lorem",
  "  - family: Dolor",
  "    given: Sit",
  "  container-title: Journal of Examples",
  "  volume: 3",
  "  issue: 2",
  "  page: 1-9",
  "  issued: 2020",
  "  DOI: 10.1000/example.2020.3",
  "- id: amet2019",
  "  type: book",
  "  title: The book of elit",
  "  author:",
  "  - family: Amet",
  "    given: Consectetur",
  "  publisher: Example Press",
  "  publisher-place: Berlin",
  "  issued: 2019",
  "- id: tempor2018",
  "  type: thesis",
  "  title: Labore et dolore at scale",
  "  genre: PhD thesis",
  "  author:",
  "  - family: Tempor",
  "    given: Incididunt",
  "  publisher: Example University",
  "  issued: 2018",
  "- id: magna2021",
  "  type: webpage",
  "  title: Ut enim ad minim veniam",
  "  author:",
  "  - family: Magna",
  "    given: Aliqua",
  "  publisher: Example Site",
  "  issued: 2021",
  "  URL: https://example.org/ut-enim",
];

/** The keys the example works have, in file order. */
export const EXAMPLE_KEYS = [
  "ipsum2020",
  "amet2019",
  "tempor2018",
  "magna2021",
];

/**
 * The template: a text that uses every form the parser reads, a footnote
 * that cites, and the block where the list goes. `nl` is the document's
 * line break.
 */
export function templateText(nl: string): string {
  return [
    "# Lorem ipsum",
    "",
    "Lorem ipsum dolor sit amet [@ipsum2020], consectetur adipiscing elit " +
      "[@amet2019, p. 3]. Sed do eiusmod tempor incididunt " +
      "[see @tempor2018, chap. 2, emphasis added] ut labore et dolore magna " +
      "aliqua [@ipsum2020; @amet2019].",
    "",
    "@magna2021 says ut enim ad minim veniam, and Tempor [-@tempor2018, p. 7] " +
      "agrees. Quis nostrud exercitation ullamco laboris[^note] nisi ut aliquip.",
    "",
    "[^note]: A footnote cites too [@amet2019].",
    "",
    "## References",
    "",
    "::: {#refs}",
    ":::",
    "",
  ].join(nl);
}

const TRUISH = /^(t|true|yes)$/i;

/**
 * The dialog's values for a header: the keys the features read, each as the
 * header has it. `hasReferences` says whether an inline list is there, which
 * the dialog then leaves alone.
 */
export function setupFromHeader(header: string): {
  setup: CitationSetup;
  hasReferences: boolean;
} {
  const lines = header.split(/\r\n?|\n/);
  const scalar = (key: string): string => {
    const re = new RegExp(`^${key}\\s*:\\s*(.*)$`);
    for (const line of lines) {
      const m = re.exec(line);
      if (m !== null) return unquoteYaml(m[1].trim());
    }
    return "";
  };
  const list = (key: string): string[] => {
    const at = lines.findIndex((l) => new RegExp(`^${key}\\s*:\\s*$`).test(l));
    if (at === -1) return [];
    const items: string[] = [];
    for (let i = at + 1; i < lines.length; i++) {
      const m = /^\s*-\s+(.*)$/.exec(lines[i]);
      if (m === null) break;
      items.push(unquoteYaml(m[1].trim()));
    }
    return items;
  };
  const one = scalar("bibliography");
  const files = one !== "" ? [one] : list("bibliography");
  return {
    setup: {
      style: scalar("csl") || DEFAULT_STYLE,
      lang: scalar("lang"),
      bibliography: files.join(", "),
      inlineReferences: false,
      sectionTitle: scalar("reference-section-title"),
      suppressBibliography: TRUISH.test(scalar("suppress-bibliography")),
      refsegment: TRUISH.test(scalar("refsegment")),
      template: false,
    },
    hasReferences: lines.some((l) => /^references\s*:/.test(l)),
  };
}

function unquoteYaml(value: string): string {
  return value.replace(/^"(.*)"$|^'(.*)'$/, "$1$2");
}

/**
 * The header lines for the dialog's values: a key the dialog leaves empty
 * is removed, so that the header says only what the dialog shows. The
 * `references:` key is added only when asked for and never removed here.
 * With the template and a folder to write into, the example file joins the
 * bibliography; without a folder the example works go inline instead.
 */
export function headerEntries(
  setup: CitationSetup,
  hasFolder: boolean,
): Record<string, HeaderValue> {
  const files = setup.bibliography
    .split(",")
    .map((f) => f.trim())
    .filter((f) => f !== "");
  let inline = setup.inlineReferences;
  if (setup.template) {
    if (hasFolder) {
      if (!files.includes(EXAMPLE_FILE)) files.push(EXAMPLE_FILE);
    } else {
      inline = true;
    }
  }
  const entries: Record<string, HeaderValue> = {
    csl: setup.style || DEFAULT_STYLE,
    lang: setup.lang || null,
    bibliography:
      files.length === 0
        ? null
        : files.length === 1
          ? files[0]
          : files.map((f) => `- ${f}`),
    "reference-section-title": setup.sectionTitle || null,
    "suppress-bibliography": setup.suppressBibliography ? "true" : null,
    refsegment: setup.refsegment ? "true" : null,
  };
  if (inline) entries.references = EXAMPLE_REFERENCES;
  return entries;
}

/** The keys the template cites, for the test that pins them to the works. */
export function templateKeys(): string[] {
  const keys = findCitations(templateText("\n")).flatMap((c) =>
    c.items.map((item) => item.key),
  );
  return [...new Set(keys)];
}
