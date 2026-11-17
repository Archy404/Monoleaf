/**
 * Live citations in the writing view: `[@key]` reads as the style prints it,
 * "(Doe & Roe, 2020)", while the cursor is off its line, as a footnote
 * marker does; on the line the source comes back for editing. The text is
 * the backend's rendering of the whole document, so numeric styles count in
 * document order; it is kept until the citations or the header change. A
 * key the references do not have stays as typed, marked like a broken link.
 * Citations inside code and in the header are left alone.
 */
import { invoke } from "@tauri-apps/api/core";
import {
  EditorState,
  RangeSetBuilder,
  StateEffect,
  type Extension,
} from "@codemirror/state";
import {
  Decoration,
  DecorationSet,
  EditorView,
  ViewPlugin,
  ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { findCitations, type ParsedCitation } from "./citationsyntax";
import type { RenderedCitations } from "./export";
import { getCurrentDocumentPath } from "./localimages";
import { frontMatter } from "./meta";
import { ancestorMatches } from "./syntax";

/** Renders a document's citations with its header; the backend by default. */
export type LiveRenderer = (
  header: string,
  citations: readonly ParsedCitation[],
) => Promise<RenderedCitations>;

const fromBackend: LiveRenderer = (header, citations) =>
  invoke<RenderedCitations>("render_citations", {
    documentPath: getCurrentDocumentPath(),
    frontMatter: header,
    citations,
  });

/** How long typing may pause before the backend is asked again; the pause
 * of the reference loader, so both settle together. */
const RENDER_DELAY_MS = 400;

/** Ask the backend again although nothing in the document changed: a
 * bibliography file changed on disk. */
export const refreshLiveCitations = StateEffect.define<null>();

const rendered = StateEffect.define<{
  key: string;
  answers: readonly (string | null)[];
}>();

interface Found {
  from: number;
  to: number;
  citation: ParsedCitation;
}

const CODE_NODE = /Code|Comment|HTML/;
const FRONT_MATTER_RE = /^---\r?\n[\s\S]*?\r?\n---/;

/** Every citation of the document in order, as the export finds them: line
 * by line, code and the header skipped. */
function collect(state: EditorState): Found[] {
  const headerEnd =
    FRONT_MATTER_RE.exec(state.doc.sliceString(0, 1 << 16))?.[0].length ?? 0;
  const found: Found[] = [];
  for (let n = 1; n <= state.doc.lines; n++) {
    const line = state.doc.line(n);
    if (line.to <= headerEnd) continue;
    for (const citation of findCitations(line.text)) {
      const from = line.from + citation.from;
      if (ancestorMatches(state, from, CODE_NODE)) continue;
      found.push({ from, to: line.from + citation.to, citation });
    }
  }
  return found;
}

class CitationWidget extends WidgetType {
  constructor(
    readonly text: string,
    readonly source: string,
  ) {
    super();
  }
  eq(other: CitationWidget) {
    return other.text === this.text && other.source === this.source;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-citation";
    span.textContent = this.text;
    span.title = this.source;
    return span;
  }
  // A click lands the cursor on the line, which reveals the source.
  ignoreEvent() {
    return false;
  }
}

/** The citation's text without the markup some styles add. */
function plainText(html: string): string {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  return parsed.body.textContent ?? "";
}

const unknownMark = Decoration.mark({ class: "cm-citation-unknown" });

function cursorOnLine(state: EditorState, from: number, to: number): boolean {
  const lineStart = state.doc.lineAt(from).from;
  const lineEnd = state.doc.lineAt(to).to;
  return state.selection.ranges.some(
    (r) => r.head >= lineStart && r.head <= lineEnd,
  );
}

class LiveCitations {
  decorations: DecorationSet = Decoration.none;
  atomics: DecorationSet = Decoration.none;
  private found: Found[];
  /** The header and citations of the document as they are now. */
  private currentKey = "";
  /** The request the answers belong to. */
  private answeredKey = "";
  private answers: readonly (string | null)[] = [];
  /** The last answer per citation source, to bridge an edit elsewhere until
   * the new answer arrives. */
  private bySource = new Map<string, string | null>();
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly view: EditorView,
    private readonly render: LiveRenderer,
    private readonly delay: number,
  ) {
    this.found = collect(view.state);
    this.currentKey = this.keyOf(view.state);
    this.schedule();
    this.rebuild();
  }

  update(update: ViewUpdate) {
    let answered = false;
    for (const tr of update.transactions) {
      for (const effect of tr.effects) {
        if (effect.is(rendered)) {
          this.answeredKey = effect.value.key;
          this.answers = effect.value.answers;
          this.remember();
          answered = true;
        } else if (effect.is(refreshLiveCitations)) {
          this.answeredKey = "";
          this.schedule();
        }
      }
    }
    if (update.docChanged) {
      this.found = collect(update.state);
      this.currentKey = this.keyOf(update.state);
      this.schedule();
    }
    if (
      answered ||
      update.docChanged ||
      update.viewportChanged ||
      update.selectionSet
    ) {
      this.rebuild();
    }
  }

  destroy() {
    clearTimeout(this.timer);
  }

  private keyOf(state: EditorState): string {
    const header = frontMatter(state.doc.sliceString(0, 1 << 16));
    return JSON.stringify([header, this.found.map((f) => f.citation)]);
  }

  private remember() {
    if (this.answeredKey !== this.currentKey) return;
    this.found.forEach((f, i) => {
      this.bySource.set(sourceOf(f), this.answers[i] ?? null);
    });
  }

  private schedule() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.request(), this.delay);
  }

  private request() {
    const key = this.currentKey;
    if (key === this.answeredKey || this.found.length === 0) return;
    const header = frontMatter(this.view.state.doc.sliceString(0, 1 << 16));
    const citations = this.found.map((f) => f.citation);
    this.render(header, citations).then(
      (result) => {
        this.view.dispatch({
          effects: rendered.of({ key, answers: result.citations }),
        });
      },
      () => {
        // The backend refused (an unknown style): the sources stay visible.
      },
    );
  }

  /** What to show for the citation at `index`: its text, null for an
   * unknown key, undefined while no answer fits. */
  private textFor(index: number): string | null | undefined {
    if (this.answeredKey === this.currentKey) return this.answers[index];
    return this.bySource.get(sourceOf(this.found[index]));
  }

  private rebuild() {
    const decorations = new RangeSetBuilder<Decoration>();
    const atomics = new RangeSetBuilder<Decoration>();
    const state = this.view.state;
    this.found.forEach((f, index) => {
      if (
        !this.view.visibleRanges.some((r) => f.from <= r.to && f.to >= r.from)
      )
        return;
      if (cursorOnLine(state, f.from, f.to)) return;
      const text = this.textFor(index);
      if (text === undefined) return;
      if (text === null) {
        decorations.add(f.from, f.to, unknownMark);
        return;
      }
      const source = state.doc.sliceString(f.from, f.to);
      const deco = Decoration.replace({
        widget: new CitationWidget(plainText(text), source),
      });
      decorations.add(f.from, f.to, deco);
      atomics.add(f.from, f.to, deco);
    });
    this.decorations = decorations.finish();
    this.atomics = atomics.finish();
  }
}

function sourceOf(found: Found): string {
  return JSON.stringify(found.citation.items) + String(found.citation.inText);
}

/** The live citations; `render` and `delay` are for tests. */
export function liveCitations(
  render: LiveRenderer = fromBackend,
  delay = RENDER_DELAY_MS,
): Extension {
  const plugin = ViewPlugin.define(
    (view) => new LiveCitations(view, render, delay),
    { decorations: (p) => p.decorations },
  );
  return [
    plugin,
    EditorView.atomicRanges.of(
      (view) => view.plugin(plugin)?.atomics ?? Decoration.none,
    ),
  ];
}
