/**
 * The references the open document names in its header, as the Rust side
 * reads them (src-tauri/src/references.rs): the list that `@` completion and
 * the hover show. Loaded once per document and header; typing in the body
 * never touches it, and a document without references never asks the backend.
 */

import { invoke } from "@tauri-apps/api/core";
import { frontMatter } from "./meta";

/** COUPLED: `ReferenceSummary` and `References` in src-tauri/src/references.rs. */
export interface ReferenceSummary {
  key: string;
  authors: string[];
  year: number | null;
  title: string;
}

export interface References {
  entries: ReferenceSummary[];
  warnings: string[];
}

type Loader = (path: string | null, header: string) => Promise<References>;

const NONE: References = { entries: [], warnings: [] };
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

export function resetReferencesForTests(): void {
  current = NONE;
  loadedPath = null;
  loadedHeader = null;
}
