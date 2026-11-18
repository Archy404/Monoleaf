// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { ensureSyntaxTree } from "@codemirror/language";
import { markdownForMode } from "./portability";
import {
  liveCitations,
  refreshLiveCitations,
  type LiveRenderer,
} from "./livecitations";

const ANSWERS: Record<string, string | null> = {
  doe2020: "(Doe 2020)",
  roe: "Roe (2019, p. 3)",
  nobody: null,
};

/** A backend that answers by key and records what it was asked. */
function stub() {
  const calls: string[][] = [];
  const render: LiveRenderer = (_header, citations) => {
    calls.push(citations.map((c) => c.items[0].key));
    return Promise.resolve({
      citations: citations.map((c) => ANSWERS[c.items[0].key] ?? null),
      bibliography: [
        { key: "doe2020", label: "[1]", html: "Doe, J. <i>A Study</i>." },
      ],
      hangingIndent: true,
      title: "References",
      warnings: [],
    });
  };
  return { calls, render };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 10));

// A header that names references: without one a citation is text.
const REFS = "---\nbibliography: r.bib\n---\n\n";

describe("live citations", () => {
  const views: EditorView[] = [];
  afterEach(() => {
    for (const view of views) view.destroy();
    views.length = 0;
  });

  function open(doc: string, render: LiveRenderer) {
    const parent = document.createElement("div");
    document.body.appendChild(parent);
    const state = EditorState.create({
      doc,
      extensions: [markdownForMode("enhanced"), liveCitations(render, 0)],
    });
    ensureSyntaxTree(state, doc.length, 5000);
    const view = new EditorView({ state, parent });
    views.push(view);
    return view;
  }

  const shown = (view: EditorView) =>
    Array.from(view.contentDOM.querySelectorAll(".cm-citation")).map(
      (e) => e.textContent,
    );

  const DOC =
    "---\ntitle: T\nbibliography: r.bib\n---\n\nSee [@doe2020] here.\n\nAnd @roe [p. 3] there, `[@doe2020]` in code.\n";

  it("asks for nothing and marks nothing when the header names no references", async () => {
    const { calls, render } = stub();
    const view = open(
      "Ping @bob [on Slack](https://x.example) and [@doe2020].\n",
      render,
    );
    await settle();
    expect(calls).toEqual([]);
    expect(shown(view)).toEqual([]);
    expect(view.contentDOM.querySelector(".cm-citation-unknown")).toBeNull();
  });

  it("shows the rendering off the cursor's line and the source on it", async () => {
    const { calls, render } = stub();
    const view = open(DOC, render);
    await settle();
    expect(calls).toEqual([["doe2020", "roe"]]);
    expect(shown(view)).toEqual(["(Doe 2020)", "Roe (2019, p. 3)"]);
    view.dispatch({ selection: { anchor: DOC.indexOf("[@doe") } });
    expect(shown(view)).toEqual(["Roe (2019, p. 3)"]);
    expect(view.contentDOM.textContent).toContain("See [@doe2020] here.");
  });

  it("marks a key the references do not have and keeps its source", async () => {
    const view = open(
      REFS + "Text [@nobody] here.\n\nNext line.\n",
      stub().render,
    );
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    await settle();
    expect(shown(view)).toEqual([]);
    const marked = view.contentDOM.querySelector(".cm-citation-unknown");
    expect(marked?.textContent).toBe("[@nobody]");
  });

  it("asks the backend again only when the citations or the header change", async () => {
    const { calls, render } = stub();
    const view = open(DOC, render);
    await settle();
    view.dispatch({ selection: { anchor: 2 } });
    view.dispatch({ changes: { from: DOC.length, insert: "More text.\n" } });
    await settle();
    expect(calls).toHaveLength(1);
    view.dispatch({
      changes: { from: view.state.doc.length, insert: "Again [@roe].\n" },
    });
    await settle();
    expect(calls).toHaveLength(2);
    expect(calls[1]).toEqual(["doe2020", "roe", "roe"]);
    view.dispatch({ changes: { from: 4, to: 12, insert: "csl: apa" } });
    await settle();
    expect(calls).toHaveLength(3);
    view.dispatch({ effects: refreshLiveCitations.of(null) });
    await settle();
    expect(calls).toHaveLength(4);
  });

  it("keeps the last text of an unchanged citation while a new answer is pending", async () => {
    const { calls, render } = stub();
    const view = open(DOC, render);
    await settle();
    const slow: LiveRenderer = () => new Promise(() => {});
    const view2 = open(DOC, slow);
    expect(shown(view2)).toEqual([]);
    view.dispatch({
      changes: { from: view.state.doc.length, insert: "New [@nobody].\n" },
    });
    expect(shown(view)).toEqual(["(Doe 2020)", "Roe (2019, p. 3)"]);
    await settle();
    expect(calls).toHaveLength(2);
  });

  it("drops an answer that arrives after a newer request was made", async () => {
    type Answer = Awaited<ReturnType<LiveRenderer>>;
    const pending: ((answer: Answer) => void)[] = [];
    const render: LiveRenderer = () =>
      new Promise((resolve) => pending.push(resolve));
    const doc = REFS + "See [@doe2020].\n\n::: {#refs}\n:::\n";
    const view = open(doc, render);
    await settle();
    view.dispatch({ changes: { from: doc.length, insert: "And [@roe].\n" } });
    await settle();
    expect(pending).toHaveLength(2);
    const answer = (label: string, n: number): Answer => ({
      citations: Array.from({ length: n }, () => "(x)"),
      bibliography: [{ key: "doe2020", label: null, html: label }],
      hangingIndent: false,
      title: null,
      warnings: [],
    });
    pending[1](answer("newer", 2));
    await settle();
    pending[0](answer("older", 1));
    await settle();
    view.dispatch({ selection: { anchor: 0 } });
    expect(view.contentDOM.querySelector(".cm-refs")?.textContent).toBe(
      "newer",
    );
  });

  it("shows the reference list at the refs block and its source on it", async () => {
    const doc = REFS + "See [@doe2020].\n\n::: {#refs}\n\n:::\n\nAfter.\n";
    const view = open(doc, stub().render);
    await settle();
    const list = view.contentDOM.querySelector(".cm-refs");
    expect(list?.className).toBe("cm-refs cm-refs-hanging");
    // The header's title belongs to the list without a block, as in pandoc.
    expect(view.contentDOM.textContent).not.toContain("References");
    expect(list?.querySelector(".cm-refs-entry")?.innerHTML).toBe(
      '<span class="cm-refs-label">[1]</span><span>Doe, J. <i>A Study</i>.</span>',
    );
    expect(view.contentDOM.textContent).not.toContain(":::");
    view.dispatch({ selection: { anchor: doc.indexOf("#refs") } });
    expect(view.contentDOM.querySelector(".cm-refs")).toBeNull();
    expect(view.contentDOM.textContent).toContain("::: {#refs}");
  });

  it("shows the list at every block, after what a block holds", async () => {
    const doc =
      REFS +
      "[@doe2020]\n\n::: {#refs}\n:::\n\n::: {#refs}\nWorks cited:\n:::\n\nEnd.\n";
    const view = open(doc, stub().render);
    view.dispatch({ selection: { anchor: doc.length } });
    await settle();
    expect(view.contentDOM.querySelectorAll(".cm-refs")).toHaveLength(2);
    expect(view.contentDOM.textContent).toContain("Works cited:");
    // The cursor on the second block's closing fence reveals only that one.
    view.dispatch({ selection: { anchor: doc.lastIndexOf(":::") } });
    expect(view.contentDOM.querySelectorAll(".cm-refs")).toHaveLength(1);
  });

  it("lists each chapter's works at its block with refsegment", async () => {
    const render: LiveRenderer = (_header, citations) =>
      Promise.resolve({
        citations: citations.map(() => "(x)"),
        bibliography: ["a", "b", "c"].map((key) => ({
          key,
          label: null,
          html: key,
        })),
        hangingIndent: false,
        title: null,
        warnings: [],
      });
    const doc =
      "---\nbibliography: r.bib\nrefsegment: true\n---\n\nOne [@a].\n\n::: {#refs}\n:::\n\n" +
      "Two [@b].[^1]\n\n::: {#refs}\n:::\n\nThree [@a].\n\n::: {#refs}\n:::\n\n" +
      "[^1]: Note [@c].\n";
    const view = open(doc, render);
    view.dispatch({ selection: { anchor: 0 } });
    await settle();
    const lists = [...view.contentDOM.querySelectorAll(".cm-refs")].map(
      (list) => list.textContent,
    );
    expect(lists).toEqual(["a", "bc", "abc"]);
  });

  it("sends a footnote's citations at its mark and shows each answer in place", async () => {
    const sent: string[] = [];
    const render: LiveRenderer = (_header, citations) => {
      sent.push(...citations.map((c) => c.items[0].key));
      return Promise.resolve({
        citations: citations.map((_, n) => `[${n + 1}]`),
        bibliography: [],
        hangingIndent: false,
        title: null,
        warnings: [],
      });
    };
    const doc = REFS + "A [@a].[^1] B [@b].\n\n[^1]: Note [@c].\n";
    const view = open(doc, render);
    view.dispatch({ selection: { anchor: doc.length } });
    await settle();
    expect(sent).toEqual(["a", "c", "b"]);
    const shown = [...view.contentDOM.querySelectorAll(".cm-citation")].map(
      (c) => c.textContent,
    );
    expect(shown).toEqual(["[1]", "[3]", "[2]"]);
  });

  it("shows a note at every refs block when the header suppresses the list", async () => {
    const doc =
      "---\nbibliography: r.bib\nsuppress-bibliography: true\n---\n\n" +
      "See [@doe2020].\n\n::: {#refs}\n:::\n\n::: {#refs}\nWorks cited:\n:::\n\nEnd.\n";
    const view = open(doc, stub().render);
    view.dispatch({ selection: { anchor: doc.length } });
    await settle();
    const notes = view.contentDOM.querySelectorAll(".cm-refs-suppressed");
    expect(notes).toHaveLength(2);
    expect(view.contentDOM.querySelector(".cm-refs-entry")).toBeNull();
    // A block that holds text keeps its opening fence and text, as with a list.
    const text = view.contentDOM.textContent ?? "";
    expect(text.split("{#refs}")).toHaveLength(2);
    expect(text).toContain("Works cited:");
    view.dispatch({ selection: { anchor: doc.indexOf("#refs") } });
    expect(
      view.contentDOM.querySelectorAll(".cm-refs-suppressed"),
    ).toHaveLength(1);
    expect(view.contentDOM.textContent).toContain("::: {#refs}");
  });

  it("leaves a refs block alone without a list, in code and unclosed", async () => {
    const view = open(
      REFS + "No citations.\n\n::: {#refs}\n:::\n",
      stub().render,
    );
    await settle();
    expect(view.contentDOM.querySelector(".cm-refs")).toBeNull();
    const code =
      REFS +
      "[@doe2020]\n\n```\n::: {#refs}\n:::\n```\n\n::: {#refs}\n\nEnd.\n";
    const view2 = open(code, stub().render);
    view2.dispatch({ selection: { anchor: code.length } });
    await settle();
    expect(view2.contentDOM.querySelector(".cm-refs")).toBeNull();
  });
});
