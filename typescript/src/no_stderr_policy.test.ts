import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const disallowed = [
  /\bprocess\.stderr\b/,
  /\bconsole\.error\s*\(/,
  /\bstderr\.write\s*\(/,
];

describe("stderr policy", () => {
  it("runtime TypeScript sources must not use stderr", () => {
    const files = readdirSync(dir)
      .filter((name) => name.endsWith(".ts") && !name.endsWith(".test.ts"))
      .sort();

    assert.ok(files.length > 0, "no TypeScript runtime source files found");

    const violations: string[] = [];
    for (const file of files) {
      const lines = readFileSync(join(dir, file), "utf-8").split("\n");
      lines.forEach((line, idx) => {
        if (disallowed.some((rx) => rx.test(line))) {
          violations.push(`${file}:${idx + 1}: ${line.trim()}`);
        }
      });
    }

    assert.equal(
      violations.length,
      0,
      `stderr usage is disallowed:\n${violations.join("\n")}`,
    );
  });
});
