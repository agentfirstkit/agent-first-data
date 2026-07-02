import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { configFromRawArgs } from "./stream_redirect.ts";

const dir = dirname(fileURLToPath(import.meta.url));

describe("stream redirect args", () => {
  it("parses space-separated and equals values", () => {
    const config = configFromRawArgs([
      "agent-cli",
      "--stdout-file",
      "/tmp/agent-cli.out",
      "--stderr-file=/tmp/agent-cli.err",
      "ping",
    ]);
    assert.deepEqual(config, {
      stdoutFile: "/tmp/agent-cli.out",
      stderrFile: "/tmp/agent-cli.err",
    });
  });

  it("returns undefined when disabled", () => {
    assert.equal(configFromRawArgs(["agent-cli", "ping"]), undefined);
  });

  it("rejects missing values", () => {
    assert.throws(() => configFromRawArgs(["agent-cli", "--stderr-file", "--help"]));
  });

  it("redirects stdout and native stderr in a child process", () => {
    const tempDir = mkdtempSync(join(tmpdir(), "afdata-stream-redirect-"));
    const stdoutPath = join(tempDir, "stdout.log");
    const stderrPath = join(tempDir, "stderr.log");
    const childPath = join(tempDir, "redirect_child.ts");
    const tsxBin = join(dir, "..", "node_modules", ".bin", process.platform === "win32" ? "tsx.cmd" : "tsx");

    if (!existsSync(tsxBin)) {
      throw new Error(`tsx executable not found at ${tsxBin}`);
    }

    writeFileSync(
      childPath,
      [
        `import { installStreamRedirectFromRawArgs } from ${JSON.stringify(pathToFileURL(join(dir, "stream_redirect.ts")).href)};`,
        "installStreamRedirectFromRawArgs(process.argv.slice(2));",
        'process.stdout.write("stdout bytes\\n");',
        'throw new Error("stderr bytes");',
        "",
      ].join("\n"),
    );

    try {
      execFileSync(tsxBin, [childPath, "--stdout-file", stdoutPath, "--stderr-file", stderrPath], {
        encoding: "utf-8",
        stdio: "pipe",
      });
      assert.fail("child process should fail after throwing");
    } catch (error) {
      assert.ok(error instanceof Error && "stdout" in error && "stderr" in error);
      const output = error as Error & { stdout: string; stderr: string };
      assert.equal(output.stdout, "");
      assert.equal(output.stderr, "");
    }

    assert.equal(readFileSync(stdoutPath, "utf-8"), "stdout bytes\n");
    assert.match(readFileSync(stderrPath, "utf-8"), /Error: stderr bytes/);
  });
});
