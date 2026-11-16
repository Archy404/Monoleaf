import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import {
  currentReferences,
  forgetLoadedReferences,
  resetReferencesForTests,
  syncReferences,
  type References,
} from "./references";

const found = {
  entries: [{ key: "doe", authors: [], year: null, title: "" }],
  warnings: [],
};
const withBib = "---\ntitle: T\nbibliography: refs.bib\n---\n\nbody\n";
const calls = (command: string) =>
  invokeMock.mock.calls.filter((call) => call[0] === command);

describe("syncReferences", () => {
  beforeEach(() => {
    resetReferencesForTests();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(found);
  });

  it("never asks the backend for a document without reference keys", async () => {
    expect(await syncReferences(null, "# No header\n")).toEqual({
      entries: [],
      warnings: [],
    });
    expect(await syncReferences("/d/a.md", "---\ntitle: T\n---\n")).toEqual({
      entries: [],
      warnings: [],
    });
    expect(calls("load_references")).toHaveLength(0);
  });

  it("loads once per document and header, and again when either changes", async () => {
    expect(await syncReferences("/d/a.md", withBib)).toEqual(found);
    expect(invokeMock).toHaveBeenCalledWith("load_references", {
      documentPath: "/d/a.md",
      frontMatter: "title: T\nbibliography: refs.bib",
    });
    await syncReferences("/d/a.md", withBib.replace("body", "typed"));
    expect(calls("load_references")).toHaveLength(1);
    await syncReferences("/d/a.md", withBib.replace("refs", "other"));
    await syncReferences("/d/b.md", withBib.replace("refs", "other"));
    expect(calls("load_references")).toHaveLength(3);
  });

  it("has the backend watch the files of every header it sees, even one without files", async () => {
    await syncReferences("/d/a.md", withBib);
    await syncReferences("/d/a.md", "---\ntitle: T\n---\n");
    expect(calls("watch_references").map((call) => call[1])).toEqual([
      {
        documentPath: "/d/a.md",
        frontMatter: "title: T\nbibliography: refs.bib",
      },
      { documentPath: "/d/a.md", frontMatter: "title: T" },
    ]);
  });

  it("loads the same header again after a reference file changed", async () => {
    await syncReferences("/d/a.md", withBib);
    forgetLoadedReferences();
    await syncReferences("/d/a.md", withBib);
    expect(calls("load_references")).toHaveLength(2);
  });

  it("drops a result that a newer header overtook", async () => {
    let finishFirst: (refs: References) => void = () => {};
    invokeMock.mockReturnValueOnce(
      new Promise<References>((resolve) => (finishFirst = resolve)),
    );
    const first = syncReferences("/d/a.md", withBib);
    expect(
      await syncReferences("/d/a.md", withBib.replace("refs", "other")),
    ).toEqual(found);
    finishFirst({ ...found, warnings: ["stale"] });
    await first;
    expect(currentReferences()).toEqual(found);
  });

  it("turns a failed call into a warning", async () => {
    // The watch call comes first and must not eat the one rejection.
    invokeMock.mockImplementation((command: string) =>
      command === "load_references"
        ? Promise.reject(new Error("no backend"))
        : Promise.resolve(undefined),
    );
    expect(await syncReferences("/d/a.md", withBib)).toEqual({
      entries: [],
      warnings: ["references: Error: no backend"],
    });
  });
});
