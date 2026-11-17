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
    "---\ntitle: T\n---\n\nSee [@doe2020] here.\n\nAnd @roe [p. 3] there, `[@doe2020]` in code.\n";

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
    const view = open("Text [@nobody] here.\n\nNext line.\n", stub().render);
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

  it("shows the reference list at the refs block and its source on it", async () => {
    const doc = "See [@doe2020].\n\n::: {#refs}\n\n:::\n\nAfter.\n";
    const view = open(doc, stub().render);
    await settle();
    const list = view.contentDOM.querySelector(".cm-refs");
    expect(list?.className).toBe("cm-refs cm-refs-hanging");
    expect(list?.querySelector(".cm-refs-title")?.textContent).toBe(
      "References",
    );
    expect(list?.querySelector(".cm-refs-entry")?.innerHTML).toBe(
      '<span class="cm-refs-label">[1]</span><span>Doe, J. <i>A Study</i>.</span>',
    );
    expect(view.contentDOM.textContent).not.toContain(":::");
    view.dispatch({ selection: { anchor: doc.indexOf("#refs") } });
    expect(view.contentDOM.querySelector(".cm-refs")).toBeNull();
    expect(view.contentDOM.textContent).toContain("::: {#refs}");
  });

  it("leaves a refs block alone without a list and when it holds text", async () => {
    const view = open("No citations.\n\n::: {#refs}\n:::\n", stub().render);
    await settle();
    expect(view.contentDOM.querySelector(".cm-refs")).toBeNull();
    const view2 = open("[@doe2020]\n\n::: {#refs}\ntext\n:::\n", stub().render);
    view2.dispatch({ selection: { anchor: 0 } });
    await settle();
    expect(view2.contentDOM.querySelector(".cm-refs")).toBeNull();
  });
});
