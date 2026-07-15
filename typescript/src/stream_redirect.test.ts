import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, statSync, symlinkSync, writeFileSync } from "node:fs";
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
    // fd-level stdout/stderr redirection is a POSIX capability; Node on Windows
    // cannot reassign the process stdio fds the way this relies on.
    if (process.platform === "win32") return;
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
    writeFileSync(stdoutPath, "existing stdout\n");

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

    assert.equal(readFileSync(stdoutPath, "utf-8"), "existing stdout\nstdout bytes\n");
    assert.match(readFileSync(stderrPath, "utf-8"), /Error: stderr bytes/);
    if (process.platform !== "win32") {
      assert.equal(statSync(stderrPath).mode & 0o777, 0o600);
    }
  });

  it("rejects symbolic link targets", () => {
    if (process.platform === "win32") return;
    const tempDir = mkdtempSync(join(tmpdir(), "afdata-stream-redirect-symlink-"));
    const realPath = join(tempDir, "real.log");
    const symlinkPath = join(tempDir, "stdout.log");
    const childPath = join(tempDir, "redirect_symlink_child.ts");
    const tsxBin = join(dir, "..", "node_modules", ".bin", "tsx");

    writeFileSync(realPath, "");
    symlinkSync(realPath, symlinkPath);
    writeFileSync(
      childPath,
      [
        `import { installStreamRedirectFromRawArgs } from ${JSON.stringify(pathToFileURL(join(dir, "stream_redirect.ts")).href)};`,
        "installStreamRedirectFromRawArgs(process.argv.slice(2));",
        'process.stdout.write("should not redirect\\n");',
        "",
      ].join("\n"),
    );

    assert.throws(
      () => execFileSync(tsxBin, [childPath, "--stdout-file", symlinkPath], { encoding: "utf-8", stdio: "pipe" }),
      /symbolic link/,
    );
    assert.equal(readFileSync(realPath, "utf-8"), "");
  });
});
