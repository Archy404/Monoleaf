import { describe, expect, it } from "vitest";
import { findCitations, type CitedItem } from "./citationsyntax";

const item = (key: string, rest: Partial<CitedItem> = {}): CitedItem => ({
  key,
  prefix: "",
  suffix: "",
  suppressAuthor: false,
  ...rest,
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

  it("leaves e-mail addresses, plain brackets and empty keys alone", () => {
    expect(
      findCitations("write to jane@doe.org [not a citation] or @ me"),
    ).toEqual([]);
  });

  it("reads the keys of a bracket that is no citation list as in-text", () => {
    const found = findCitations("[@doe2020; not a key]");
    expect(found.map((c) => [c.inText, c.items[0].key])).toEqual([
      [true, "doe2020"],
    ]);
  });
});
