"""Tests for stdout/stderr stream redirection argument handling."""

import os
import sys

import pytest

from agent_first_data.stream_redirect import StreamRedirectConfig, config_from_raw_args, install


def test_config_from_raw_args_space_and_equals_values() -> None:
    config = config_from_raw_args(
        [
            "agent-cli",
            "--stdout-file",
            "/tmp/agent-cli.out",
            "--stderr-file=/tmp/agent-cli.err",
            "ping",
        ]
    )
    assert config is not None
    assert config.stdout_file == "/tmp/agent-cli.out"
    assert config.stderr_file == "/tmp/agent-cli.err"


def test_config_from_raw_args_disabled() -> None:
    assert config_from_raw_args(["agent-cli", "ping"]) is None


def test_config_from_raw_args_missing_value() -> None:
    with pytest.raises(ValueError):
        config_from_raw_args(["agent-cli", "--stderr-file", "--help"])


def test_install_redirects_and_restores_output(tmp_path) -> None:
    stdout_path = tmp_path / "stdout.log"
    stderr_path = tmp_path / "stderr.log"
    stdout_path.write_bytes(b"existing stdout\n")

    redirect = install(StreamRedirectConfig(stdout_file=str(stdout_path), stderr_file=str(stderr_path)))
    try:
        os.write(sys.stdout.fileno(), b"stdout bytes\n")
        os.write(sys.stderr.fileno(), b"stderr bytes\n")
    finally:
        redirect.close()

    assert stdout_path.read_bytes() == b"existing stdout\nstdout bytes\n"
    assert stderr_path.read_bytes() == b"stderr bytes\n"
    assert (stderr_path.stat().st_mode & 0o777) == 0o600


def test_install_rejects_symlink_targets(tmp_path) -> None:
    real_path = tmp_path / "real.log"
    symlink_path = tmp_path / "stdout.log"
    real_path.write_bytes(b"")
    try:
        symlink_path.symlink_to(real_path)
    except OSError as exc:
        pytest.skip(f"symlink unsupported: {exc}")

    with pytest.raises(OSError, match="symbolic link"):
        install(StreamRedirectConfig(stdout_file=str(symlink_path)))
