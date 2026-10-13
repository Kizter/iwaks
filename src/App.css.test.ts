import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The stylesheet is read from disk rather than imported: vitest stubs CSS
 * imports by default (`css: false`), so `./App.css?raw` arrives as an empty
 * string. The path is resolved from the working directory, which is the project
 * root for both `npm test` and a bare `npx vitest`.
 */
const rawCss = readFileSync(join(process.cwd(), "src", "App.css"), "utf8");

/**
 * The stylesheet with comments blanked out.
 *
 * Necessary, not cosmetic: this file has comments containing `{` and `}` (ASCII
 * section dividers and column sketches), and a regex that counts braces reads
 * those as block boundaries, then pairs every selector with the wrong body.
 */
const css = rawCss.replace(/\/\*[\s\S]*?\*\//g, " ");

/**
 * Every rule, as its selector list and its declarations.
 *
 * A brace-counting scan rather than a regex, because `@media` blocks nest rules
 * and only the inner ones matter here — the outer prelude is an at-rule, not a
 * selector.
 */
function rules(): Array<{ selectors: string[]; body: string }> {
  const out: Array<{ selectors: string[]; body: string }> = [];
  let depth = 0;
  let headerStart = 0;
  let header = "";
  let bodyStart = 0;
  for (let i = 0; i < css.length; i++) {
    if (css[i] === "{") {
      if (depth === 0) {
        header = css.slice(headerStart, i);
        bodyStart = i + 1;
      }
      depth++;
    } else if (css[i] === "}") {
      depth--;
      if (depth === 0) {
        out.push({
          selectors: header.split(",").map((s) => s.trim()),
          body: css.slice(bodyStart, i),
        });
        headerStart = i + 1;
      }
    }
  }
  return out;
}

/** The declaration of one property inside one rule block. */
function declares(selector: string, property: string): string {
  const pattern = new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*([^;]+)`, "m");
  for (const rule of rules()) {
    if (!rule.selectors.includes(selector)) continue;
    const value = pattern.exec(rule.body);
    if (value) return value[1].trim();
  }
  throw new Error(`no "${property}" declared for ${selector}`);
}

/**
 * Issue #05 turned three manual "look at it and confirm" boxes into these.
 *
 * jsdom has no layout engine, so a test cannot measure where "Time" ended up at
 * 1440px — anything claiming to have verified the pixels would be theatre. What
 * *can* be verified, and what actually caused the bug, is the invariant behind
 * it: a list header and its rows must read their column tracks from the same
 * declaration. When the header had its own literal, "Time" landed in the `1fr`
 * track instead of the last one.
 *
 * So these assertions read the stylesheet. That is unusual, and deliberate: the
 * alternative is a comment saying "verified once by eye", which is exactly what
 * the issue had before.
 */
/** The value a custom property holds in `:root`. */
function token(name: string): string {
  const root = rules().find((r) => r.selectors.includes(":root") && r.body.includes(name));
  if (!root) throw new Error(`${name} is not defined in :root`);
  const found = new RegExp(`${name}\\s*:\\s*([^;]+)`).exec(root.body);
  return found![1].trim();
}



describe("list column tracks (issue #05)", () => {
  it("gives the track header and its rows the same tracks", () => {
    // Both must go through the token: a literal in either one is the drift.
    expect(declares(".track-head", "grid-template-columns")).toBe("var(--track-cols)");
    expect(declares(".track-row", "grid-template-columns")).toBe("var(--track-cols)");
  });

  it("gives the queue header and its rows the same tracks", () => {
    expect(declares(".queue-head", "grid-template-columns")).toBe("var(--queue-cols)");
    expect(declares(".queue-row", "grid-template-columns")).toBe("var(--queue-cols)");
  });

  it("keeps the queue on four columns, which is what its rows have", () => {
    // The original defect: a 3-column header over 4-column rows, so the Queue's
    // "Time" had nowhere to go and was auto-placed into the `1fr` track.
    const tracks = token("--queue-cols").split(/\s+/);
    expect(tracks, "position, cover, main, meta").toHaveLength(4);
    expect(tracks[3]).toBe("240px");
  });

  it("pins the duration header to the last track instead of letting it float", () => {
    // With the tracks shared, "Time" still needs to be told which one it is;
    // auto-placement is what put it in `1fr` when the counts disagreed.
    expect(declares(".queue-head-dur", "grid-column")).toBe("4");
    expect(declares(".track-head-dur", "text-align")).toBe("right");
  });

  it("reserves the header's leading cell so the labels land on the right tracks", () => {
    // The header prints "TITLE … TIME" while the rows print
    // "position, cover, title, duration". The hidden first span takes the row
    // number's column, which is why it must exist.
    expect(css).toMatch(/\.track-head\s*>\s*span:first-child\s*\{[^}]*visibility:\s*hidden/);
    expect(css).toMatch(
      /\.queue-head\s*>\s*span:first-child,\s*\.queue-head\s*>\s*span:nth-child\(2\)\s*\{[^}]*visibility:\s*hidden/,
    );
  });
});

describe("dark mode palette (issue #25)", () => {
  it("keeps every colour literal inside the two palette blocks", () => {
    // The rule the project set itself: colour literals live in the palette, and
    // a stray one in a component rule is invisible until it has to be fixed in
    // two places.
    const palettes = [
      ...css.matchAll(/^[^{}]*data-theme[^{}]*\{[\s\S]*?\n\}/gm),
    ].map((m) => m[0]);
    // The light palette is written as `:root, :root[data-theme="light"]`, so the
    // selector list has to be matched whole rather than one selector at a time.
    expect(palettes.length, "one palette block per theme").toBe(2);
    const outside = palettes.reduce((rest, block) => rest.replace(block, ""), css);
    const literals = outside.match(/#[0-9a-fA-F]{3,8}\b|\brgba?\(|hsla?\(/g) ?? [];
    expect(literals, `colour literals outside the palette: ${literals.join(", ")}`).toEqual(
      [],
    );
  });
});