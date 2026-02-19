"""Tests for AFDATA logging module."""

import json
import logging
import sys
from io import StringIO
from unittest.mock import patch

from agent_first_data.afdata_logging import AfdataHandler, AfdataJsonHandler, init_json, init_plain, init_yaml, span, get_logger


def capture_log(fn):
    """Run fn and return the parsed JSON log line."""
    buf = StringIO()
    with patch("sys.stdout", buf):
        fn()
    line = buf.getvalue().strip()
    assert line, "No log output captured"
    return json.loads(line)


def make_logger(name="test"):
    """Create a fresh logger with AfdataJsonHandler."""
    logger = logging.getLogger(name)
    logger.handlers = [AfdataJsonHandler()]
    logger.setLevel(logging.DEBUG)
    return logger


class TestBasicFields:
    def test_info_message(self):
        logger = make_logger("test_basic")
        m = capture_log(lambda: logger.info("hello world"))
        assert m["message"] == "hello world"
        assert m["code"] == "info"
        assert "timestamp_epoch_ms" in m
        assert m["target"] == "test_basic"

    def test_warning_code(self):
        logger = make_logger("test_warn")
        m = capture_log(lambda: logger.warning("something wrong"))
        assert m["code"] == "warn"

    def test_error_code(self):
        logger = make_logger("test_error")
        m = capture_log(lambda: logger.error("failure"))
        assert m["code"] == "error"

    def test_debug_code(self):
        logger = make_logger("test_debug")
        m = capture_log(lambda: logger.debug("verbose"))
        assert m["code"] == "debug"


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

        m = capture_log(lambda: adapter.info("ready", extra={"code": "log", "event": "startup"}))
        assert m["code"] == "log"
        assert m["event"] == "startup"


class TestGetLogger:
    def test_default_fields(self):
        # Ensure root logger has AfdataJsonHandler
        root = logging.getLogger()
        root.handlers = [AfdataJsonHandler()]
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
        assert "code=info" in output

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
