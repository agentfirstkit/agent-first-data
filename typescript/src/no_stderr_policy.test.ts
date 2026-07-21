import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));

// Ad-hoc stderr is forbidden in runtime sources. The one sanctioned exception is
// the CliEmitter's own diagnostic sink: in finite mode `error`/`progress`/`log`
// envelopes flow to stderr *through the emitter*, and those lines carry the
// `stderr-sink` marker comment. Bypassing the emitter with a stray stderr write
// still fails, and `console.error` is never sanctioned.
const SANCTION = "stderr-sink";
const alwaysForbidden = [/\bconsole\.error\s*\(/];
const stderrForbidden = [/\bprocess\.stderr\b/, /\bstderr\.write\s*\(/];

function violatesStderrPolicy(line: string): boolean {
  if (alwaysForbidden.some((rx) => rx.test(line))) return true;
  return stderrForbidden.some((rx) => rx.test(line)) && !line.includes(SANCTION);
}

describe("stderr policy", () => {
  it("runtime TypeScript sources must not use ad-hoc stderr", () => {
    const files = readdirSync(dir)
      .filter((name) => name.endsWith(".ts") && !name.endsWith(".test.ts"))
      .sort();

    assert.ok(files.length > 0, "no TypeScript runtime source files found");

    const violations: string[] = [];
    for (const file of files) {
      const lines = readFileSync(join(dir, file), "utf-8").split("\n");
      lines.forEach((line, idx) => {
        if (violatesStderrPolicy(line)) {
          violations.push(`${file}:${idx + 1}: ${line.trim()}`);
        }
      });
    }

    assert.equal(
      violations.length,
      0,
      `ad-hoc stderr usage is disallowed:\n${violations.join("\n")}`,
    );
  });

  it("sanctions the emitter diagnostic sink but still rejects stray stderr", () => {
    // The emitter's blessed diagnostic sink passes because it carries the marker.
    assert.equal(
      violatesStderrPolicy("  process.stderr.write(line); // stderr-sink: CliEmitter diagnostic channel"),
      false,
    );
    // A bare stderr write bypassing the emitter still fails.
    assert.equal(violatesStderrPolicy("process.stderr.write(x)"), true);
    assert.equal(violatesStderrPolicy("const s = process.stderr;"), true);
    assert.equal(violatesStderrPolicy("stderr.write(x)"), true);
    // console.error is never sanctioned, even with the marker.
    assert.equal(violatesStderrPolicy("console.error('x'); // stderr-sink"), true);
  });
});
