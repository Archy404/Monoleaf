/**
 * Citations in the text, pandoc style: `@key` on its own or inside brackets
 * (`[@doe2020, p. 3]`, `[see @doe2020; -@roe]`). The editor needs to know
 * whether a position sits in a citation key, for `@` completion and for the
 * hover; nothing is rendered here.
 */

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
