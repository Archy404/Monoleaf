/**
 * Live citations in the writing view: `[@key]` reads as the style prints it,
 * "(Doe & Roe, 2020)", while the cursor is off its line, as a footnote
 * marker does; on the line the source comes back for editing. The text is
 * the backend's rendering of the whole document, so numeric styles count in
 * document order; it is kept until the citations or the header change. A
 * key the references do not have stays as typed, marked like a broken link.
 * Citations inside code and in the header are left alone. Every
 * `::: {#refs}` block shows the reference list the same way, as a block
 * widget, after what the block holds, as the export places it.
 */
import { invoke } from "@tauri-apps/api/core";
import {
  EditorState,
  RangeSetBuilder,
  StateEffect,
  StateField,
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
import {
  findCitations,
  REFS_CLOSE_RE,
  REFS_OPEN_RE,
  refsegmentOn,
  segmentLists,
  type ParsedCitation,
  type SegmentEvent,
} from "./citationsyntax";
import { footnoteLabelAt, footnoteRefPos } from "./footnotes";
import type { RenderedCitations } from "./export";
import { getCurrentDocumentPath } from "./localimages";
import { frontMatter } from "./meta";
import { sanitizeDocumentHtml } from "./sanitize";
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
  result: RenderedCitations;
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

/** A `::: {#refs}` block: where the list shows and the lines that reveal
 * the source when the cursor is on them. An empty block is replaced whole;
 * a block that holds text keeps it, and its closing fence shows the list. */
interface RefsBlock {
  /** The opening fence, where the block stands among the citations. */
  at: number;
  from: number;
  to: number;
}

/** Every closed refs block outside code, in document order. */
function findRefsBlocks(state: EditorState): RefsBlock[] {
  const blocks: RefsBlock[] = [];
  for (let n = 1; n <= state.doc.lines; n++) {
    const line = state.doc.line(n);
    if (!REFS_OPEN_RE.test(line.text)) continue;
    if (ancestorMatches(state, line.from, CODE_NODE)) continue;
    let empty = true;
    for (let m = n + 1; m <= state.doc.lines; m++) {
      const next = state.doc.line(m);
      if (REFS_CLOSE_RE.test(next.text)) {
        blocks.push({
          at: line.from,
          from: empty ? line.from : next.from,
          to: next.to,
        });
        n = m;
        break;
      }
      if (next.text.trim() !== "") empty = false;
    }
  }
  return blocks;
}

class ReferenceListWidget extends WidgetType {
  constructor(readonly list: RenderedCitations) {
    super();
  }
  eq(other: ReferenceListWidget) {
    return (
      other.list.hangingIndent === this.list.hangingIndent &&
      JSON.stringify(other.list.bibliography) ===
        JSON.stringify(this.list.bibliography)
    );
  }
  toDOM() {
    const block = document.createElement("div");
    block.className = this.list.hangingIndent
      ? "cm-refs cm-refs-hanging"
      : "cm-refs";
    for (const entry of this.list.bibliography) {
      const row = document.createElement("div");
      row.className = "cm-refs-entry";
      if (entry.label !== null) {
        const label = document.createElement("span");
        label.className = "cm-refs-label";
        label.textContent = plainText(entry.label);
        row.append(label);
      }
      const text = document.createElement("span");
      // The backend's HTML (italics in titles), through the same sanitizer
      // as the export's HTML before it reaches the app's DOM.
      text.innerHTML = sanitizeDocumentHtml(entry.html);
      row.append(text);
      block.append(row);
    }
    return block;
  }
  ignoreEvent() {
    return false;
  }
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
          this.answers = effect.value.result.citations;
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
    const state = this.view.state;
    const header = frontMatter(state.doc.sliceString(0, 1 << 16));
    // The backend numbers in the order it gets the citations: a footnote's
    // stand at its mark, as pandoc reads them.
    const at = this.found.map((f) => markPosition(state, f));
    const order = this.found
      .map((_, i) => i)
      .sort((a, b) => at[a] - at[b] || this.found[a].from - this.found[b].from);
    const citations = order.map((i) => this.found[i].citation);
    this.render(header, citations).then(
      (sent) => {
        const back: (string | null)[] = [];
        order.forEach((i, n) => (back[i] = sent.citations[n] ?? null));
        const result = { ...sent, citations: back };
        this.view.dispatch({ effects: rendered.of({ key, result }) });
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
    const state = this.view.state;
    const visible = (from: number, to: number) =>
      this.view.visibleRanges.some((r) => from <= r.to && to >= r.from);
    const planned: {
      from: number;
      to: number;
      deco: Decoration;
      atomic: boolean;
    }[] = [];
    this.found.forEach((f, index) => {
      if (!visible(f.from, f.to)) return;
      if (cursorOnLine(state, f.from, f.to)) return;
      const text = this.textFor(index);
      if (text === undefined) return;
      if (text === null) {
        planned.push({
          from: f.from,
          to: f.to,
          deco: unknownMark,
          atomic: false,
        });
        return;
      }
      const source = state.doc.sliceString(f.from, f.to);
      const deco = Decoration.replace({
        widget: new CitationWidget(plainText(text), source),
      });
      planned.push({ from: f.from, to: f.to, deco, atomic: true });
    });
    planned.sort((a, b) => a.from - b.from);
    const decorations = new RangeSetBuilder<Decoration>();
    const atomics = new RangeSetBuilder<Decoration>();
    for (const p of planned) {
      decorations.add(p.from, p.to, p.deco);
      if (p.atomic) atomics.add(p.from, p.to, p.deco);
    }
    this.decorations = decorations.finish();
    this.atomics = atomics.finish();
  }
}

/** The list at every refs block. A block widget must come from a state
 * field, not a view plugin; the field keeps the last list the backend
 * rendered and replaces each block's lines unless the cursor is on them.
 * The header's title is not shown: pandoc never puts it into a block. */
const referenceList = StateField.define<{
  list: RenderedCitations | null;
  blocks: RefsBlock[];
  /** What each block lists, recomputed when the text changes. */
  keys: (ReadonlySet<string> | null)[];
  decorations: DecorationSet;
}>({
  create: (state) => {
    const blocks = findRefsBlocks(state);
    return {
      list: null,
      blocks,
      keys: blockLists(state, blocks),
      decorations: Decoration.none,
    };
  },
  update(value, tr) {
    let { list, blocks, keys } = value;
    for (const effect of tr.effects) {
      if (effect.is(rendered)) list = effect.value.result;
    }
    if (tr.docChanged) {
      blocks = findRefsBlocks(tr.state);
      keys = blockLists(tr.state, blocks);
    }
    let decorations = Decoration.none;
    if (list !== null && list.bibliography.length > 0) {
      const ranges = blocks.flatMap((block, n) => {
        if (cursorOnLine(tr.state, block.from, block.to)) return [];
        const own = keys[n] ?? null;
        const shown =
          own === null
            ? list
            : {
                ...list,
                bibliography: list.bibliography.filter((e) => own.has(e.key)),
              };
        if (shown.bibliography.length === 0) return [];
        const widget = new ReferenceListWidget(shown);
        return [
          Decoration.replace({ widget, block: true }).range(
            block.from,
            block.to,
          ),
        ];
      });
      decorations = Decoration.set(ranges);
    }
    return { list, blocks, keys, decorations };
  },
  provide: (field) => [
    EditorView.decorations.from(field, (value) => value.decorations),
    EditorView.atomicRanges.of((view) => view.state.field(field).decorations),
  ],
});

/** Where a citation stands in reading order: a footnote's citation at the
 * footnote's first mark, any other where it is written. */
function markPosition(state: EditorState, found: Found): number {
  const label = footnoteLabelAt(state, found.from);
  const mark = label === null ? null : footnoteRefPos(state, label);
  return mark ?? found.from;
}

/** What each block lists, as the export decides it: every work, or with
 * `refsegment` its chapter's works; a footnote's citations count at its
 * first mark. */
function blockLists(
  state: EditorState,
  blocks: readonly RefsBlock[],
): (ReadonlySet<string> | null)[] {
  const header = frontMatter(state.doc.sliceString(0, 1 << 16));
  if (!refsegmentOn(header)) return blocks.map(() => null);
  const placed: { at: number; event: SegmentEvent }[] = collect(state).map(
    (f) => {
      const keys = f.citation.items.map((item) => item.key);
      return { at: markPosition(state, f), event: { kind: "cite", keys } };
    },
  );
  for (const block of blocks)
    placed.push({ at: block.at, event: { kind: "block" } });
  placed.sort((a, b) => a.at - b.at);
  return segmentLists(
    placed.map((p) => p.event),
    true,
  );
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
    referenceList,
    EditorView.atomicRanges.of(
      (view) => view.plugin(plugin)?.atomics ?? Decoration.none,
    ),
  ];
}
