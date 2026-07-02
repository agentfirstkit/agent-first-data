/**
 * Optional stdout/stderr file redirection for AFDATA CLIs.
 *
 * This redirects Node's process stdout/stderr file descriptors to append-only
 * files. It is not an AFDATA formatter and does not convert stderr diagnostics
 * into JSON.
 */

import { closeSync, openSync } from "node:fs";

export const STDOUT_FILE_ARG = "--stdout-file";
export const STDERR_FILE_ARG = "--stderr-file";

export interface StreamRedirectConfig {
  stdoutFile?: string;
  stderrFile?: string;
}

export interface InstalledStreamRedirect {
  stdoutFile?: string;
  stderrFile?: string;
  processLifetime: true;
}

let installed = false;
const fillerFds: number[] = [];

export function configFromRawArgs(args: readonly string[]): StreamRedirectConfig | undefined {
  const config: StreamRedirectConfig = {};
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]!;
    if (arg === "--") break;
    if (arg === STDOUT_FILE_ARG) {
      const [value, next] = takeValue(args, i, STDOUT_FILE_ARG);
      config.stdoutFile = value;
      i = next;
    } else if (arg.startsWith(`${STDOUT_FILE_ARG}=`)) {
      config.stdoutFile = arg.slice(STDOUT_FILE_ARG.length + 1);
    } else if (arg === STDERR_FILE_ARG) {
      const [value, next] = takeValue(args, i, STDERR_FILE_ARG);
      config.stderrFile = value;
      i = next;
    } else if (arg.startsWith(`${STDERR_FILE_ARG}=`)) {
      config.stderrFile = arg.slice(STDERR_FILE_ARG.length + 1);
    }
  }
  validateConfig(config);
  return config.stdoutFile === undefined && config.stderrFile === undefined ? undefined : config;
}

export function installStreamRedirectFromRawArgs(args = process.argv.slice(2)): InstalledStreamRedirect | undefined {
  const config = configFromRawArgs(args);
  return config === undefined ? undefined : installStreamRedirect(config);
}

export function installStreamRedirect(config: StreamRedirectConfig): InstalledStreamRedirect | undefined {
  validateConfig(config);
  if (config.stdoutFile === undefined && config.stderrFile === undefined) {
    return undefined;
  }
  if (installed) {
    throw new Error("stream redirection already installed");
  }
  installed = true;

  // Validate both files before changing fd 1/2. Node does not expose dup2, so
  // this helper is intended for process-lifetime CLI setup rather than scoped
  // redirection that can be restored later.
  validateWritable(config.stdoutFile);
  validateWritable(config.stderrFile);
  redirectFd(1, config.stdoutFile);
  redirectFd(2, config.stderrFile);

  return {
    stdoutFile: config.stdoutFile,
    stderrFile: config.stderrFile,
    processLifetime: true,
  };
}

function validateConfig(config: StreamRedirectConfig): void {
  if (config.stdoutFile === "") {
    throw new Error("--stdout-file must not be empty");
  }
  if (config.stderrFile === "") {
    throw new Error("--stderr-file must not be empty");
  }
}

function takeValue(args: readonly string[], idx: number, flag: string): [string, number] {
  const next = idx + 1;
  const value = args[next];
  if (value === undefined || value.startsWith("--")) {
    throw new Error(`${flag} requires a value`);
  }
  return [value, next];
}

function validateWritable(path: string | undefined): void {
  if (path === undefined) return;
  const fd = openSync(path, "a");
  closeSync(fd);
}

function redirectFd(targetFd: 1 | 2, path: string | undefined): void {
  if (path === undefined) return;
  fillLowerFds(targetFd);
  try {
    closeSync(targetFd);
  } catch {
    // The target fd may already be closed in unusual embedding environments.
  }
  const fd = openSync(path, "a");
  if (fd !== targetFd) {
    closeSync(fd);
    throw new Error(`failed to redirect fd ${targetFd} to ${path}`);
  }
}

function fillLowerFds(targetFd: 1 | 2): void {
  const nullDevice = process.platform === "win32" ? "NUL" : "/dev/null";
  while (true) {
    const fd = openSync(nullDevice, "r");
    if (fd < targetFd) {
      fillerFds.push(fd);
      continue;
    }
    closeSync(fd);
    return;
  }
}
