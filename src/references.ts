/**
 * The references the open document names in its header, as the Rust side
 * reads them (src-tauri/src/references.rs): the list that `@` completion and
 * the hover show. Loaded once per document and header; typing in the body
 * never touches it, and a document without references never asks the backend.
 */

import { invoke } from "@tauri-apps/api/core";
import { frontMatter } from "./meta";

/** COUPLED: `ReferenceSummary`, `Source` and `LoadedSummaries` in src-tauri/src/references.rs. */
export interface ReferenceSummary {
  key: string;
  authors: string[];
  year: number | null;
  title: string;
  format: ReferenceFormat;
}

/** What a reference was read from: a `.bib` file, or CSL data (a JSON or
 * YAML file, the header's `references:` list). */
export type ReferenceFormat = "bib" | "csl";

/**
 * How a document mixes the two formats. The backend renders each format in
 * a run of its own, so "sources" is a caution and "cited" a degraded export:
 * numbered and listed one format after the other, and two works that cite
 * alike are not told apart.
 */
export type FormatMix = "none" | "sources" | "cited";

export function formatMix(
  refs: References,
  citedKeys: Iterable<string>,
): FormatMix {
  const byKey = new Map<string, ReferenceFormat>();
  for (const entry of refs.entries) {
    if (!byKey.has(entry.key)) byKey.set(entry.key, entry.format);
  }
  if (new Set(byKey.values()).size < 2) return "none";
  const cited = new Set<ReferenceFormat>();
  for (const key of citedKeys) {
    const format = byKey.get(key);
    if (format !== undefined) cited.add(format);
  }
  return cited.size === 2 ? "cited" : "sources";
}

/** The panel's line for a mix, null when there is none. */
export function formatMixNote(mix: FormatMix): string | null {
  switch (mix) {
    case "none":
      return null;
    case "sources":
      return ".bib and CSL references together: the export renders each format in a run of its own";
    case "cited":
      return "the text cites .bib and CSL references: the export numbers and lists each format on its own, and two works that cite alike are not told apart";
  }
}

/** Where entries came from: `references:` for the inline list, else the
 * path as written in the header; zero entries where the source failed. */
export interface Source {
  name: string;
  entries: number;
}

export interface References {
  entries: ReferenceSummary[];
  warnings: string[];
  sources: Source[];
}

type Loader = (path: string | null, header: string) => Promise<References>;

const NONE: References = { entries: [], warnings: [], sources: [] };
// The header keys that name references, at column 0 as pandoc reads them.
const NAMES_REFERENCES = /^(?:bibliography|references)\s*:/m;

let current = NONE;
let loadedPath: string | null = null;
let loadedHeader: string | null = null;

export function currentReferences(): References {
  return current;
}

const fromBackend: Loader = (documentPath, frontMatter) =>
  invoke<References>("load_references", { documentPath, frontMatter });

/** Have the backend watch the files this header names, for `path`. */
type Watcher = (path: string | null, header: string) => Promise<void>;

const watchBackend: Watcher = (documentPath, frontMatter) =>
  invoke("watch_references", { documentPath, frontMatter });

/**
 * Bring the list in line with the document at `path` and its `text`. Only a
 * changed path or header loads again, and only a header that names references
 * asks the backend. A failed call becomes a warning, so the editor goes on
 * with an empty list. Returns the list that is current afterwards.
 */
export async function syncReferences(
  path: string | null,
  text: string,
  load: Loader = fromBackend,
  watch: Watcher = watchBackend,
): Promise<References> {
  const header = frontMatter(text);
  if (path === loadedPath && header === loadedHeader) return current;
  loadedPath = path;
  loadedHeader = header;
  // Always, so that a header that stopped naming files ends their watches. A
  // watch that cannot be set only means a change goes unnoticed.
  void watch(path, header).catch(() => {});
  if (!NAMES_REFERENCES.test(header)) {
    current = NONE;
    return current;
  }
  const loaded = await load(path, header).catch((e: unknown) => ({
    entries: [],
    warnings: [`references: ${String(e)}`],
    sources: [],
  }));
  // A newer header or document overtook this load; its result is stale.
  if (path !== loadedPath || header !== loadedHeader) return current;
  current = loaded;
  return current;
}

/** A reference file changed on disk: the next sync loads again, same header. */
export function forgetLoadedReferences(): void {
  loadedHeader = null;
}

function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** The status-bar entry: "" for a document without references, else the
 * counts, "4 references, 1 warning". */
export function referencesLabel(refs: References): string {
  if (refs.sources.length === 0 && refs.warnings.length === 0) return "";
  const parts = [count(refs.entries.length, "reference", "references")];
  if (refs.warnings.length > 0)
    parts.push(count(refs.warnings.length, "warning", "warnings"));
  return parts.join(", ");
}

/** One source of the panel's list with the warnings that name it. */
export interface SourceLine {
  name: string;
  summary: string;
  warnings: string[];
}

/** The panel's lines: every source in header order, then any source only a
 * warning names (the header when its YAML did not parse). A warning loses the
 * name it starts with, since its line carries that. */
export function describeSources(refs: References): SourceLine[] {
  const lines: SourceLine[] = refs.sources.map((source) => ({
    name: source.name,
    summary: count(source.entries, "entry", "entries"),
    warnings: [],
  }));
  for (const warning of refs.warnings) {
    const name = warning.slice(0, warning.indexOf(": "));
    let line = lines.find((l) => l.name === name);
    if (line === undefined) {
      line = { name: name || "other", summary: "", warnings: [] };
      lines.push(line);
    }
    line.warnings.push(warning.slice(name.length + 2));
  }
  for (const line of lines) {
    if (line.summary === "0 entries" && line.warnings.length > 0)
      line.summary = "not read";
  }
  return lines;
}

export function resetReferencesForTests(): void {
  current = NONE;
  loadedPath = null;
  loadedHeader = null;
}
