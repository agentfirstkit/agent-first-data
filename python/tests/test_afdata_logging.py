"""Tests for AFDATA logging module."""

import json
import logging
import sys
from io import StringIO
from unittest.mock import patch

from agent_first_data.afdata_logging import AfdataHandler, init_json, init_plain, init_yaml, span, get_logger
from agent_first_data.format import RedactionOptions


def capture_log(fn):
    """Run fn and return the parsed JSON log line."""
    buf = StringIO()
    with patch("sys.stdout", buf):
        fn()
    line = buf.getvalue().strip()
    assert line, "No log output captured"
    return json.loads(line)


def make_logger(name="test"):
    """Create a fresh logger with AfdataHandler."""
    logger = logging.getLogger(name)
    logger.handlers = [AfdataHandler()]
    logger.setLevel(logging.DEBUG)
    return logger


class TestBasicFields:
    def test_info_message(self):
        logger = make_logger("test_basic")
        m = capture_log(lambda: logger.info("hello world"))
        assert m["message"] == "hello world"
        assert m["code"] == "log"
        assert m["level"] == "info"
        assert "timestamp_epoch_ms" in m
        assert "target" not in m

    def test_warning_code(self):
        logger = make_logger("test_warn")
        m = capture_log(lambda: logger.warning("something wrong"))
        assert m["code"] == "log"
        assert m["level"] == "warn"

    def test_error_code(self):
        logger = make_logger("test_error")
        m = capture_log(lambda: logger.error("failure"))
        assert m["code"] == "log"
        assert m["level"] == "error"

    def test_debug_code(self):
        logger = make_logger("test_debug")
        m = capture_log(lambda: logger.debug("verbose"))
        assert m["code"] == "log"
        assert m["level"] == "debug"


class TestSpan:
    def test_span_adds_fields(self):
        logger = make_logger("test_span")

        def run():
            with span(request_id="abc-123"):
                logger.info("processing")

        m = capture_log(run)
        assert m["request_id"] == "abc-123"
        assert m["message"] == "processing"

    def test_nested_spans(self):
        logger = make_logger("test_nested")

        def run():
            with span(request_id="outer"):
                with span(step="inner"):
                    logger.info("nested")

        m = capture_log(run)
        assert m["request_id"] == "outer"
        assert m["step"] == "inner"

    def test_inner_span_overrides_parent(self):
        logger = make_logger("test_override")

        def run():
            with span(source="parent"):
                with span(source="child"):
                    logger.info("test")

        m = capture_log(run)
        assert m["source"] == "child"

    def test_span_fields_removed_after_exit(self):
        logger = make_logger("test_exit")
        buf = StringIO()

        with patch("sys.stdout", buf):
            with span(request_id="temp"):
                logger.info("inside")
            buf2 = StringIO()

        with patch("sys.stdout", buf2):
            logger.info("outside")

        outside = json.loads(buf2.getvalue().strip())
        assert "request_id" not in outside


class TestCodeOverride:
    def test_explicit_code(self):
        logger = make_logger("test_code")
        adapter = get_logger("test_code")

        m = capture_log(lambda: adapter.info("ready", extra={"code": "ignored", "event": "startup"}))
        assert m["code"] == "log"
        assert m["level"] == "info"
        assert m["event"] == "startup"

    def test_exception_field_is_readable(self):
        logger = make_logger("test_exc")
        adapter = get_logger("test_exc")
        m = capture_log(lambda: adapter.error("request failed", extra={"error": Exception("timeout")}))
        assert m["error"] == "timeout"


class TestRedactionOptions:
    def test_legacy_secret_names_apply_to_all_formats(self):
        for format in ("json", "plain", "yaml"):
            logger = logging.getLogger(f"test_redaction_{format}")
            logger.handlers = [
                AfdataHandler(
                    format=format,
                    redaction=RedactionOptions(secret_names=("authorization",)),
                )
            ]
            logger.setLevel(logging.DEBUG)
            logger.propagate = False
            adapter = get_logger(f"test_redaction_{format}")

            output = capture_raw(
                lambda: adapter.info(
                    "authorization appears in message but is not name-redacted",
                    extra={
                        "authorization": "Bearer legacy",
                        "request_url": "https://example.test/path?authorization=legacy&ok=1",
                    },
                )
            )
            assert "***" in output
            assert "Bearer legacy" not in output
            assert "authorization=legacy" not in output
            assert "authorization appears in message" in output

    def test_legacy_secret_names_are_redacted(self):
        logger = logging.getLogger("test_redaction")
        logger.handlers = [
            AfdataHandler(redaction=RedactionOptions(secret_names=("authorization",)))
        ]
        logger.setLevel(logging.DEBUG)
        adapter = get_logger("test_redaction")

        m = capture_log(
            lambda: adapter.info(
                "authorization appears in message but is not name-redacted",
                extra={
                    "authorization": "Bearer legacy",
                    "request_url": "https://example.test/path?authorization=legacy&ok=1",
                },
            )
        )
        assert m["authorization"] == "***"
        assert "authorization appears in message" in m["message"]
        assert "authorization=***" in m["request_url"]
        assert "authorization=legacy" not in m["request_url"]

    def test_default_redaction_leaves_legacy_names_visible(self):
        logger = make_logger("test_redaction_default")
        adapter = get_logger("test_redaction_default")

        m = capture_log(lambda: adapter.info("request", extra={"authorization": "Bearer visible"}))
        assert m["authorization"] == "Bearer visible"

    def test_init_accepts_secret_names(self):
        buf = StringIO()
        with patch("sys.stdout", buf):
            init_json("DEBUG", secret_names=("authorization",))
            adapter = get_logger("test_redaction_init")
            adapter.info("request", extra={"authorization": "Bearer legacy"})

        m = json.loads(buf.getvalue().strip())
        assert m["authorization"] == "***"


class TestGetLogger:
    def test_default_fields(self):
        # Ensure root logger has AfdataHandler
        root = logging.getLogger()
        root.handlers = [AfdataHandler()]
        root.setLevel(logging.DEBUG)

        adapter = get_logger("test_adapter", component="myservice")

        m = capture_log(lambda: adapter.info("event"))
        assert m["component"] == "myservice"
        assert m["message"] == "event"


def capture_raw(fn):
    """Run fn and return the raw output string."""
    buf = StringIO()
    with patch("sys.stdout", buf):
        fn()
    return buf.getvalue()


class TestPlainFormat:
    def test_plain_output(self):
        logger = logging.getLogger("test_plain")
        logger.handlers = [AfdataHandler(format="plain")]
        logger.setLevel(logging.DEBUG)

        output = capture_raw(lambda: logger.info("hello"))
        # Plain format is single-line logfmt
        assert "message=" in output
        assert "code=log" in output
        assert "level=info" in output

    def test_init_plain(self):
        buf = StringIO()
        with patch("sys.stdout", buf):
            init_plain("DEBUG")
            logging.getLogger("test_init_plain").info("test")
        output = buf.getvalue()
        assert "message=" in output


class TestYamlFormat:
    def test_yaml_output(self):
        logger = logging.getLogger("test_yaml")
        logger.handlers = [AfdataHandler(format="yaml")]
        logger.setLevel(logging.DEBUG)

        output = capture_raw(lambda: logger.info("hello"))
        # YAML format starts with ---
        assert output.startswith("---")

    def test_init_yaml(self):
        buf = StringIO()
        with patch("sys.stdout", buf):
            init_yaml("DEBUG")
            logging.getLogger("test_init_yaml").info("test")
        output = buf.getvalue()
        assert output.startswith("---")
