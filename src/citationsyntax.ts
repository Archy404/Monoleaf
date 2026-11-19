/**
 * Pandoc's citation syntax, read from plain text: `[@a]`, `[@a; @b]`,
 * `[see @a, p. 3; also -@b]` and in-text `@a` or `@a [p. 3]`. Pure text
 * functions, shared by the editor and the export; nothing is rendered here.
 */

/** A citation key around a position: `from` is the `@`, `to` the key's end. */
export interface Citation {
  from: number;
  to: number;
  key: string;
}

// A pandoc key starts with a letter, digit or `_`, in any script, and goes on
// with those and internal punctuation; punctuation at the very end belongs to
// the sentence.
/** A header line that names references: a `bibliography:` file or an inline
 * `references:` list, at column 0 as pandoc reads them. */
const NAMES_REFERENCES = /^(?:bibliography|references)\s*:/m;

/** Whether the document asks for citations at all. Without references in
 * the header, `@name` and `[@name]` are plain text, as they were before. */
export function namesReferences(header: string): boolean {
  return NAMES_REFERENCES.test(header);
}

export const KEY_START = /[\p{L}\p{N}_]/u;
const KEY_CHAR = /[\p{L}\p{N}_:.#$%&\-+?<>~/]/u;

/** Whether the `@` at `at` opens a key: one that follows a letter or digit is
 * part of an e-mail address; one after a backslash is escaped, as in pandoc
 * and as Markdown reads `\@`. */
function opensKey(text: string, at: number): boolean {
  return at === 0 || !(KEY_START.test(text[at - 1]) || text[at - 1] === "\\");
}

/** The `}` that balances the `{` at `open`, or -1: pandoc reads no braced
 * key across a space or without its closing brace. */
function closingBrace(text: string, open: number): number {
  let depth = 0;
  for (let i = open; i < text.length && !/\s/.test(text[i]); i++) {
    if (text[i] === "{") {
      depth++;
    } else if (text[i] === "}" && --depth === 0) {
      return i;
    }
  }
  return -1;
}

/**
 * The citation whose key spans `pos`, or null. The key may be empty when the
 * cursor sits right after a fresh `@`, which is what completion waits for.
 * A braced key, `@{key}`, holds any characters but spaces; `from` and `to`
 * include the braces, `key` does not.
 */
export function citationAt(text: string, pos: number): Citation | null {
  const brace = text.lastIndexOf("@{", pos - 1) + 1;
  const close = brace === 0 ? -1 : closingBrace(text, brace);
  if (close !== -1 && brace <= pos && pos <= close + 1) {
    if (!opensKey(text, brace - 1)) {
      return null;
    }
    return {
      from: brace - 1,
      to: close + 1,
      key: text.slice(brace + 1, close),
    };
  }
  let start = pos;
  while (start > 0 && KEY_CHAR.test(text[start - 1])) start--;
  const at = start - 1;
  if (text[at] !== "@") return null;
  if (!opensKey(text, at)) {
    return null;
  }
  if (start < text.length && !KEY_START.test(text[start]) && start < pos)
    return null;
  let end = pos;
  while (end < text.length && KEY_CHAR.test(text[end])) end++;
  while (end > pos && !KEY_START.test(text[end - 1])) end--;
  if (end === start && start < text.length && KEY_CHAR.test(text[start]))
    return null;
  return { from: at, to: end, key: text.slice(start, end) };
}

/** One cited work inside a citation, with what the citation says about it.
 * The backend reads a locator, "p. 3", off the start of the suffix: the
 * terms it knows come from the citation locale, not from here. */
export interface CitedItem {
  key: string;
  /** Text before the key: "see" in `[see @a]`. */
  prefix: string;
  /** Text after the key, without the comma: "p. 3, emphasis added". */
  suffix: string;
  /** `-@a`: the author is left out, "(2020)" in author-date styles. */
  suppressAuthor: boolean;
}

/** A whole citation in the text, brackets included in `from` and `to`. */
export interface ParsedCitation {
  from: number;
  to: number;
  items: CitedItem[];
  /** The `@a` form outside brackets: "Doe (2020)" rather than "(Doe 2020)". */
  inText: boolean;
}

/** What follows a key, without the comma that pandoc allows after it. */
function suffixOf(rest: string): string {
  return rest.replace(/^\s*,?\s*/, "").trim();
}

/** One `;`-separated part of a bracket as a cited work, or null without a key.
 * `flat` is the part with its nested brackets blanked, where the key is read. */
function parseItem(part: string, flat: string): CitedItem | null {
  for (let at = flat.indexOf("@"); at !== -1; at = flat.indexOf("@", at + 1)) {
    const found = citationAt(flat, at + 1);
    if (found === null || found.key === "") continue;
    const before = part.slice(0, found.from);
    const suppressAuthor = before.endsWith("-");
    const prefix = (suppressAuthor ? before.slice(0, -1) : before).trim();
    const suffix = suffixOf(part.slice(found.to));
    return { key: found.key, prefix, suffix, suppressAuthor };
  }
  return null;
}

/** `text` with each balanced bracket blanked to NULs of the same length. In a
 * citation a `;` or a key inside one belongs to the text, as in pandoc, and
 * the first `]` left is the citation's close. */
function blankNested(text: string): string {
  let flat = text;
  for (let last = ""; last !== flat;) {
    last = flat;
    flat = flat.replace(/\[[^[\]]*\]/g, (m) => "\0".repeat(m.length));
  }
  return flat;
}

/**
 * Every citation in `text`, in order. A bracket is a citation when each of
 * its `;`-separated parts has a key; otherwise its keys count as in-text
 * citations, as pandoc reads them. The bracket ends at the `]` that balances
 * it, so `[@a, p. [3]]` is one citation. Code spans are the caller's business.
 */
export function findCitations(text: string): ParsedCitation[] {
  const citations: ParsedCitation[] = [];
  let pos = 0;
  while (pos < text.length) {
    if (text[pos] === "[") {
      const flat = blankNested(text.slice(pos + 1));
      const length = flat.indexOf("]");
      if (length !== -1) {
        const close = pos + 1 + length;
        const flatParts = flat.slice(0, length).split(";");
        const items: CitedItem[] = [];
        let start = pos + 1;
        for (const flatPart of flatParts) {
          const part = text.slice(start, start + flatPart.length);
          start += flatPart.length + 1;
          const item = parseItem(part, flatPart);
          if (item === null) break;
          items.push(item);
        }
        if (items.length === flatParts.length) {
          citations.push({ from: pos, to: close + 1, items, inText: false });
          pos = close + 1;
          continue;
        }
      }
    } else if (text[pos] === "@") {
      const found = citationAt(text, pos + 1);
      if (found !== null && found.key !== "") {
        // the in-text form may carry its locator in a bracket: `@a [p. 3]`
        const bracket = /^ ?\[([^\]@]*)\]/.exec(text.slice(found.to));
        const suffix = suffixOf(bracket?.[1] ?? "");
        const to = found.to + (bracket?.[0].length ?? 0);
        const item = {
          key: found.key,
          prefix: "",
          suffix,
          suppressAuthor: false,
        };
        citations.push({ from: found.from, to, items: [item], inText: true });
        pos = to;
        continue;
      }
    }
    pos++;
  }
  return citations;
}

/**
 * The opening fence of the block pandoc fills with the reference list:
 * `::: {#refs}`, three or more colons, other attributes allowed after the id.
 */
export const REFS_OPEN_RE = /^:{3,}\s*\{\s*#refs(\s[^}]*)?\}\s*$/;

/** A closing fence: three or more colons alone on the line. */
export const REFS_CLOSE_RE = /^:{3,}\s*$/;

/** The classes an opening fence gives its block, `.small` as "small". */
export function refsClasses(line: string): string[] {
  const attributes = REFS_OPEN_RE.exec(line)?.[1] ?? "";
  return [...attributes.matchAll(/(?:^|\s)\.([\w-]+)/g)].map((m) => m[1]);
}

/** Whether the header sets `key` to true: the boolean, or "t", "true" or
 * "yes" in any case, quoted or not, as pandoc reads a flag; a missing key is
 * false. `key` is a literal header key.
 */
function flagOn(header: string, key: string): boolean {
  const re = new RegExp(`^${key}:[ \\t]*(.*?)[ \\t]*$`, "m");
  const value = re.exec(header)?.[1] ?? "";
  const text = value.replace(/^(["'])(.*)\1$/, "$2").toLowerCase();
  return ["t", "true", "yes"].includes(text);
}

/** Whether the header sets `refsegment`, biblatex's name for lists per
 * segment. Pandoc itself does not know the key and ignores it. */
export function refsegmentOn(header: string): boolean {
  return flagOn(header, "refsegment");
}

/** Whether the header sets `suppress-bibliography`: no reference list. */
export function suppressBibliographyOn(header: string): boolean {
  return flagOn(header, "suppress-bibliography");
}

/** A refs block or a citation, in document order; a footnote's citations
 * stand where its mark stands. */
export type SegmentEvent =
  { kind: "block" } | { kind: "cite"; keys: readonly string[] };

/**
 * What each refs block lists: null for every work, else the keys it holds.
 * Without `refsegment` every block lists every work, as in pandoc. With it,
 * a block lists the works cited since the previous block, or since the
 * start; it lists every work when nothing is cited before it in that span or
 * nothing after it at all, and the last block always lists every work.
 */
export function segmentLists(
  events: readonly SegmentEvent[],
  refsegment: boolean,
): (ReadonlySet<string> | null)[] {
  const blocks = events.flatMap((e, i) => (e.kind === "block" ? [i] : []));
  if (!refsegment) return blocks.map(() => null);
  return blocks.map((at, n) => {
    if (n === blocks.length - 1) return null;
    const start = n === 0 ? 0 : blocks[n - 1] + 1;
    const keysOf = (span: readonly SegmentEvent[]) =>
      span.flatMap((e) => (e.kind === "cite" ? e.keys : []));
    const above = keysOf(events.slice(start, at));
    const below = keysOf(events.slice(at + 1));
    return above.length === 0 || below.length === 0 ? null : new Set(above);
  });
}
