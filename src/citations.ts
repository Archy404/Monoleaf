/**
 * Citations in the text, pandoc style: `@key` on its own or inside brackets
 * (`[@doe2020, p. 3]`, `[see @doe2020; -@roe]`). The editor needs to know
 * whether a position sits in a citation key, for `@` completion and for the
 * hover; nothing is rendered here.
 */

import {
  autocompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import type { EditorState, Extension } from "@codemirror/state";
import { hoverTooltip } from "@codemirror/view";
import {
  currentReferences,
  type References,
  type ReferenceSummary,
} from "./references";

/** A citation around a position: `from` is the `@`, `to` the end of the key. */
export interface Citation {
  from: number;
  to: number;
  key: string;
}

// A pandoc key starts with a letter, digit or `_` and goes on with those and
// internal punctuation; punctuation at the very end belongs to the sentence.
const KEY_START = /[A-Za-z0-9_]/;
const KEY_CHAR = /[A-Za-z0-9_:.#$%&\-+?<>~/]/;

/**
 * The citation whose key spans `pos`, or null. The key may be empty when the
 * cursor sits right after a fresh `@`, which is what completion waits for.
 * A `@` that follows a letter or digit is part of an e-mail address.
 */
export function citationAt(text: string, pos: number): Citation | null {
  let start = pos;
  while (start > 0 && KEY_CHAR.test(text[start - 1])) start--;
  const at = start - 1;
  if (text[at] !== "@") return null;
  if (at > 0 && KEY_START.test(text[at - 1])) return null;
  if (start < text.length && !KEY_START.test(text[start]) && start < pos)
    return null;
  let end = pos;
  while (end < text.length && KEY_CHAR.test(text[end])) end++;
  while (end > pos && !KEY_START.test(text[end - 1])) end--;
  if (end === start && start < text.length && KEY_CHAR.test(text[start]))
    return null;
  return { from: at, to: end, key: text.slice(start, end) };
}

/**
 * The completions for the citation at the cursor: every reference whose key,
 * author or title contains what was typed after the `@`. The list is filtered
 * here rather than by the editor, which only matches the key.
 */
export function citationCompletions(
  context: CompletionContext,
  references: () => References = currentReferences,
): CompletionResult | null {
  const line = context.state.doc.lineAt(context.pos);
  const citation = citationAt(line.text, context.pos - line.from);
  if (citation === null) return null;
  const typed = line.text.slice(citation.from + 1, context.pos - line.from);
  const needle = typed.toLowerCase();
  const options: Completion[] = [];
  for (const entry of references().entries) {
    const haystack = [entry.key, ...entry.authors, entry.title].join(" ");
    if (!haystack.toLowerCase().includes(needle)) continue;
    const who = entry.authors[0] ?? "";
    const when = entry.year === null ? "" : String(entry.year);
    options.push({
      label: entry.key,
      detail: [who, when].filter((part) => part !== "").join(", "),
      info: entry.title,
      type: "text",
    });
  }
  if (options.length === 0) return null;
  return {
    from: line.from + citation.from + 1,
    to: line.from + citation.to,
    options,
    filter: false,
  };
}

/** `@` opens the citation list, with no icons: every entry is a reference. */
export function citationCompletion(): Extension {
  return autocompletion({ override: [citationCompletions], icons: false });
}

/** The entry as the hover shows it: who and when on one line, the title below. */
export function describeReference(entry: ReferenceSummary): string {
  const who = entry.authors.length > 0 ? entry.authors.join("; ") : "No author";
  const when = entry.year === null ? "" : ` (${entry.year})`;
  return `${who}${when}\n${entry.title || "No title"}`;
}

/** What the hover at `pos` shows and the range it belongs to, or null. */
export function citationHoverAt(
  state: EditorState,
  pos: number,
  references: () => References = currentReferences,
): { from: number; to: number; text: string } | null {
  const line = state.doc.lineAt(pos);
  const citation = citationAt(line.text, pos - line.from);
  if (citation === null || citation.key === "") return null;
  const entry = references().entries.find((e) => e.key === citation.key);
  return {
    from: line.from + citation.from,
    to: line.from + citation.to,
    text: entry ? describeReference(entry) : `${citation.key}: not found`,
  };
}

/** Hovering a citation key shows its entry; the text goes in as text only. */
export function citationHover(): Extension {
  return hoverTooltip((view, pos) => {
    const hover = citationHoverAt(view.state, pos);
    if (hover === null) return null;
    return {
      pos: hover.from,
      end: hover.to,
      above: true,
      create() {
        const dom = document.createElement("div");
        dom.className = "cm-citation-hover";
        dom.textContent = hover.text;
        return { dom };
      },
    };
  });
}
