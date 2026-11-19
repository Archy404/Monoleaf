import { describe, expect, it } from "vitest";
import {
  findCitations,
  refsegmentOn,
  segmentLists,
  type CitedItem,
  namesReferences,
  citationAt,
} from "./citationsyntax";

const item = (key: string, rest: Partial<CitedItem> = {}): CitedItem => ({
  key,
  prefix: "",
  suffix: "",
  suppressAuthor: false,
  ...rest,
});

describe("an escaped @", () => {
  it("is no citation, in the text and under the cursor", () => {
    expect(findCitations("Ping \\@bob [on Slack](x) and [@a].")).toEqual([
      { from: 29, to: 33, items: [item("a")], inText: false },
    ]);
    expect(citationAt("Ping \\@bob now", 8)).toBeNull();
    expect(citationAt("Ping @bob now", 8)).not.toBeNull();
  });
});

describe("namesReferences", () => {
  it("is true for a bibliography file or an inline list at column 0", () => {
    expect(namesReferences("title: T\nbibliography: refs.bib\n")).toBe(true);
    expect(namesReferences("references:\n- id: a\n")).toBe(true);
    expect(namesReferences("title: T\n  bibliography: x\n")).toBe(false);
    expect(namesReferences("")).toBe(false);
  });
});

describe("findCitations", () => {
  it("reads a bracketed citation with its span", () => {
    expect(findCitations("As shown [@doe2020].")).toEqual([
      { from: 9, to: 19, items: [item("doe2020")], inText: false },
    ]);
  });

  it("splits a group into prefix, key and suffix", () => {
    const [found] = findCitations(
      "[see @doe2020, pp. 3-5, emphasis added; also -@roe, chap. 2; @x]",
    );
    expect(found.items).toEqual([
      item("doe2020", { prefix: "see", suffix: "pp. 3-5, emphasis added" }),
      item("roe", { prefix: "also", suffix: "chap. 2", suppressAuthor: true }),
      item("x"),
    ]);
  });

  it("drops the comma after the key and keeps the rest as the suffix", () => {
    const [found] = findCitations("[@doe2020, 12; @roe and others]");
    expect(found.items).toEqual([
      item("doe2020", { suffix: "12" }),
      item("roe", { suffix: "and others" }),
    ]);
  });

  it("keeps a locator without the comma as the suffix", () => {
    // pandoc renders [@a p. 3] as page 3, the same as [@a, p. 3]
    const [found] = findCitations("[@a p. 3; see @b chap. 2 for this]");
    expect(found.items).toEqual([
      item("a", { suffix: "p. 3" }),
      item("b", { prefix: "see", suffix: "chap. 2 for this" }),
    ]);
  });

  it("reads the in-text form, with or without a bracket after it", () => {
    expect(findCitations("@doe2020 [p. 3] says, and @roe agrees.")).toEqual([
      {
        from: 0,
        to: 15,
        items: [item("doe2020", { suffix: "p. 3" })],
        inText: true,
      },
      { from: 26, to: 30, items: [item("roe")], inText: true },
    ]);
  });

  it("reads keys in any script, as pandoc does", () => {
    const text = "Siehe [@пункт3; @Müller2020] und @ürün_1.";
    expect(findCitations(text).map((c) => c.items.map((i) => i.key))).toEqual([
      ["пункт3", "Müller2020"],
      ["ürün_1"],
    ]);
    expect(citationAt("@пункт3", 3)).toEqual({ from: 0, to: 7, key: "пункт3" });
  });

  it("leaves e-mail addresses, plain brackets and empty keys alone", () => {
    expect(
      findCitations("write to jane@doe.org [not a citation] or @ me"),
    ).toEqual([]);
  });

  it("reads the keys of a bracket that is no citation list as in-text", () => {
    // pandoc 3.13 reads such a bracket as text with author-in-text citations
    const at = (from: number, to: number, key: string) => ({
      from,
      to,
      items: [item(key)],
      inText: true,
    });
    expect(findCitations("[@a; foo]")).toEqual([at(1, 3, "a")]);
    expect(findCitations("[foo; @a]")).toEqual([at(6, 8, "a")]);
    expect(findCitations("[@a; @b; foo]")).toEqual([
      at(1, 3, "a"),
      at(5, 7, "b"),
    ]);
    expect(findCitations("[@a, p. 3; see also]")).toEqual([at(1, 3, "a")]);
  });
});

describe("refsegment", () => {
  it("reads the flag as pandoc reads one", () => {
    expect(refsegmentOn("csl: apa\nrefsegment: true\n")).toBe(true);
    expect(refsegmentOn('refsegment: "Yes"')).toBe(true);
    expect(refsegmentOn("refsegment: false\n")).toBe(false);
    expect(refsegmentOn("csl: apa\n")).toBe(false);
    expect(refsegmentOn("  refsegment: true\n")).toBe(false);
  });

  const cite = (...keys: string[]) => ({ kind: "cite" as const, keys });
  const block = { kind: "block" as const };
  const keys = (list: ReadonlySet<string> | null) =>
    list === null ? "all" : [...list].join(",");

  it("lists every work at every block without the flag", () => {
    const events = [cite("a"), block, cite("b"), block];
    expect(segmentLists(events, false).map(keys)).toEqual(["all", "all"]);
  });

  it("lists each chapter's works and every work at the last block", () => {
    const events = [cite("a"), block, cite("b", "a"), block, cite("c"), block];
    expect(segmentLists(events, true).map(keys)).toEqual(["a", "b,a", "all"]);
  });

  it("lists every work where nothing is cited before or after a block", () => {
    const top = [block, cite("a"), block, cite("b"), block];
    expect(segmentLists(top, true).map(keys)).toEqual(["all", "a", "all"]);
    const tail = [cite("a"), block, block];
    expect(segmentLists(tail, true).map(keys)).toEqual(["all", "all"]);
  });

  it("gives the appendix after the last block to the last block's full list", () => {
    const events = [cite("a"), block, cite("b"), block, cite("c")];
    expect(segmentLists(events, true).map(keys)).toEqual(["a", "all"]);
  });
});
