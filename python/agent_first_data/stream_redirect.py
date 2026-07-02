"""Optional stdout/stderr file redirection for AFDATA CLIs."""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass
from typing import Sequence

STDOUT_FILE_ARG = "--stdout-file"
STDERR_FILE_ARG = "--stderr-file"


@dataclass(frozen=True)
class StreamRedirectConfig:
    """Resolved stdout/stderr file redirection config."""

    stdout_file: str | None = None
    stderr_file: str | None = None

    def validate(self) -> None:
        if self.stdout_file is not None and self.stdout_file == "":
            raise ValueError("--stdout-file must not be empty")
        if self.stderr_file is not None and self.stderr_file == "":
            raise ValueError("--stderr-file must not be empty")


class InstalledStreamRedirect:
    """Restores original stdout/stderr when closed."""

    def __init__(self, stdout_restore_fd: int | None, stderr_restore_fd: int | None) -> None:
        self._stdout_restore_fd = stdout_restore_fd
        self._stderr_restore_fd = stderr_restore_fd
        self._closed = False

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        sys.stdout.flush()
        sys.stderr.flush()
        if self._stdout_restore_fd is not None:
            os.dup2(self._stdout_restore_fd, sys.stdout.fileno())
            os.close(self._stdout_restore_fd)
            self._stdout_restore_fd = None
        if self._stderr_restore_fd is not None:
            os.dup2(self._stderr_restore_fd, sys.stderr.fileno())
            os.close(self._stderr_restore_fd)
            self._stderr_restore_fd = None

    def __enter__(self) -> "InstalledStreamRedirect":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


def config_from_raw_args(args: Sequence[str]) -> StreamRedirectConfig | None:
    """Resolve --stdout-file/--stderr-file from raw CLI args."""

    stdout_file: str | None = None
    stderr_file: str | None = None
    i = 0
    while i < len(args):
        arg = args[i]
        if arg == "--":
            break
        if arg == STDOUT_FILE_ARG:
            stdout_file, i = _take_value(args, i, STDOUT_FILE_ARG)
        elif arg.startswith(f"{STDOUT_FILE_ARG}="):
            stdout_file = arg.split("=", 1)[1]
        elif arg == STDERR_FILE_ARG:
            stderr_file, i = _take_value(args, i, STDERR_FILE_ARG)
        elif arg.startswith(f"{STDERR_FILE_ARG}="):
            stderr_file = arg.split("=", 1)[1]
        i += 1

    config = StreamRedirectConfig(stdout_file=stdout_file, stderr_file=stderr_file)
    config.validate()
    if config.stdout_file is None and config.stderr_file is None:
        return None
    return config


def install_from_raw_args(args: Sequence[str] | None = None) -> InstalledStreamRedirect | None:
    """Install stdout/stderr redirection from raw CLI args."""

    raw = sys.argv[1:] if args is None else args
    config = config_from_raw_args(raw)
    if config is None:
        return None
    return install(config)


def install(config: StreamRedirectConfig) -> InstalledStreamRedirect:
    """Redirect configured streams to append-only files."""

    config.validate()
    stdout_target = _prepare_target(sys.stdout.fileno(), config.stdout_file)
    try:
        stderr_target = _prepare_target(sys.stderr.fileno(), config.stderr_file)
    except Exception:
        _close_prepared(stdout_target)
        raise

    sys.stdout.flush()
    sys.stderr.flush()

    try:
        if stdout_target is not None:
            os.dup2(stdout_target.file_fd, sys.stdout.fileno())
        if stderr_target is not None:
            os.dup2(stderr_target.file_fd, sys.stderr.fileno())
    except Exception:
        if stdout_target is not None:
            os.dup2(stdout_target.restore_fd, sys.stdout.fileno())
        _close_prepared(stdout_target)
        _close_prepared(stderr_target)
        raise

    stdout_restore_fd = stdout_target.restore_fd if stdout_target is not None else None
    stderr_restore_fd = stderr_target.restore_fd if stderr_target is not None else None
    if stdout_target is not None:
        os.close(stdout_target.file_fd)
    if stderr_target is not None:
        os.close(stderr_target.file_fd)
    return InstalledStreamRedirect(stdout_restore_fd, stderr_restore_fd)


@dataclass
class _PreparedTarget:
    file_fd: int
    restore_fd: int


def _prepare_target(target_fd: int, path: str | None) -> _PreparedTarget | None:
    if path is None:
        return None
    file_fd = os.open(path, os.O_CREAT | os.O_WRONLY | os.O_APPEND, 0o666)
    try:
        restore_fd = os.dup(target_fd)
    except Exception:
        os.close(file_fd)
        raise
    return _PreparedTarget(file_fd=file_fd, restore_fd=restore_fd)


def _close_prepared(target: _PreparedTarget | None) -> None:
    if target is None:
        return
    os.close(target.file_fd)
    os.close(target.restore_fd)


def _take_value(args: Sequence[str], idx: int, flag: str) -> tuple[str, int]:
    next_idx = idx + 1
    if next_idx >= len(args) or args[next_idx].startswith("--"):
        raise ValueError(f"{flag} requires a value")
    return args[next_idx], next_idx
