import os
import subprocess
import sys
import textwrap

import rattler


def _run_tracing_script(script: str) -> subprocess.CompletedProcess[str]:
    """Run a tracing script in a subprocess (setup_tracing can only be called once per process)."""
    return subprocess.run(
        [sys.executable, "-c", textwrap.dedent(script)],
        capture_output=True,
        text=True,
        timeout=10,
    )


# -- Logging setup boilerplate shared across subprocess scripts --
_LOGGING_PREAMBLE = """
        import logging
        from rattler.rattler import setup_tracing, _emit_test_trace
        from rattler.rattler import _emit_test_trace_with_fields, _emit_test_trace_fields_only

        handler = logging.StreamHandler()
        handler.setLevel(logging.DEBUG)
        handler.setFormatter(logging.Formatter("%(name)s [%(levelname)s] %(message)s"))
        logging.root.addHandler(handler)
        logging.root.setLevel(logging.DEBUG)
"""


def _log_lines(result: subprocess.CompletedProcess[str]) -> list[str]:
    """Extract non-empty log lines from combined stdout+stderr."""
    combined = result.stdout + result.stderr
    return [line for line in combined.strip().splitlines() if line.strip()]


def test_setup_tracing_is_callable() -> None:
    assert hasattr(rattler, "setup_tracing")
    assert callable(rattler.setup_tracing)


def test_setup_tracing_invalid_filter() -> None:
    result = _run_tracing_script("""
        import rattler
        try:
            rattler.setup_tracing("invalid[[[filter")
            print("NO_ERROR")
        except ValueError as e:
            print(f"VALUE_ERROR: {e}")
        except Exception as e:
            print(f"OTHER_ERROR: {type(e).__name__}: {e}")
    """)
    assert "VALUE_ERROR:" in result.stdout, f"stdout: {result.stdout}, stderr: {result.stderr}"


def test_setup_tracing_called_twice_raises() -> None:
    result = _run_tracing_script("""
        import rattler
        rattler.setup_tracing("info")
        try:
            rattler.setup_tracing("debug")
            print("NO_ERROR")
        except RuntimeError as e:
            print(f"RUNTIME_ERROR: {e}")
        except Exception as e:
            print(f"OTHER_ERROR: {type(e).__name__}: {e}")
    """)
    assert "RUNTIME_ERROR:" in result.stdout, f"stdout: {result.stdout}, stderr: {result.stderr}"


def test_setup_tracing_bridges_to_python_logging() -> None:
    """Rust tracing events should appear in Python logging output."""
    result = _run_tracing_script(
        _LOGGING_PREAMBLE
        + """
        setup_tracing("rattler_test=info")
        _emit_test_trace("info", "hello from rust")
    """
    )
    assert _log_lines(result) == [
        "rattler.rattler_test [INFO] hello from rust",
    ]


def test_bridge_level_mapping() -> None:
    """Each Rust tracing level should map to the correct Python logging level."""
    result = _run_tracing_script(
        _LOGGING_PREAMBLE
        + """
        setup_tracing("rattler_test=trace")
        _emit_test_trace("error", "msg_error")
        _emit_test_trace("warn", "msg_warn")
        _emit_test_trace("info", "msg_info")
        _emit_test_trace("debug", "msg_debug")
        _emit_test_trace("trace", "msg_trace")
    """
    )
    assert _log_lines(result) == [
        "rattler.rattler_test [ERROR] msg_error",
        "rattler.rattler_test [WARNING] msg_warn",
        "rattler.rattler_test [INFO] msg_info",
        "rattler.rattler_test [DEBUG] msg_debug",
        "rattler.rattler_test [DEBUG] msg_trace",  # TRACE maps to DEBUG in Python
    ]


def test_bridge_respects_level_filter() -> None:
    """Events below the configured level should not appear."""
    result = _run_tracing_script(
        _LOGGING_PREAMBLE
        + """
        setup_tracing("rattler_test=warn")
        _emit_test_trace("error", "msg_error")
        _emit_test_trace("warn", "msg_warn")
        _emit_test_trace("info", "msg_info_filtered")
        _emit_test_trace("debug", "msg_debug_filtered")
    """
    )
    assert _log_lines(result) == [
        "rattler.rattler_test [ERROR] msg_error",
        "rattler.rattler_test [WARNING] msg_warn",
    ]


def test_bridge_respects_target_filter() -> None:
    """Only events matching the configured target should appear."""
    result = _run_tracing_script(
        _LOGGING_PREAMBLE
        + """
        setup_tracing("rattler_test_a=info")
        _emit_test_trace("info", "msg_target_a", target="rattler_test_a")
        _emit_test_trace("info", "msg_target_b", target="rattler_test_b")
    """
    )
    assert _log_lines(result) == [
        "rattler.rattler_test_a [INFO] msg_target_a",
    ]


def test_bridge_compound_target_filter() -> None:
    """Compound filters should enable multiple targets independently."""
    result = _run_tracing_script(
        _LOGGING_PREAMBLE
        + """
        setup_tracing("rattler_test_a=info,rattler_test_b=warn")
        _emit_test_trace("info", "a_info", target="rattler_test_a")
        _emit_test_trace("warn", "b_warn", target="rattler_test_b")
        _emit_test_trace("info", "b_info_filtered", target="rattler_test_b")
    """
    )
    assert _log_lines(result) == [
        "rattler.rattler_test_a [INFO] a_info",
        "rattler.rattler_test_b [WARNING] b_warn",
    ]


def test_bridge_logger_name_includes_target() -> None:
    """Python logger name should be 'rattler.<target>'."""
    result = _run_tracing_script(
        _LOGGING_PREAMBLE
        + """
        setup_tracing("rattler_test_a=info,rattler_test_b=info")
        _emit_test_trace("info", "from_a", target="rattler_test_a")
        _emit_test_trace("info", "from_b", target="rattler_test_b")
    """
    )
    assert _log_lines(result) == [
        "rattler.rattler_test_a [INFO] from_a",
        "rattler.rattler_test_b [INFO] from_b",
    ]


def test_setup_tracing_with_various_levels() -> None:
    for level in ["error", "warn", "info", "debug", "trace"]:
        result = _run_tracing_script(f"""
            import rattler
            rattler.setup_tracing("{level}")
            print("OK")
        """)
        assert result.returncode == 0, f"Level {level!r} failed: {result.stderr}"
        assert "OK" in result.stdout


def test_bridge_message_only() -> None:
    """Branch: message present, no extra fields -> returns message as-is."""
    result = _run_tracing_script(
        _LOGGING_PREAMBLE
        + """
        setup_tracing("rattler_test=info")
        _emit_test_trace("info", "plain message")
    """
    )
    assert _log_lines(result) == [
        "rattler.rattler_test [INFO] plain message",
    ]


def test_bridge_message_with_fields() -> None:
    """Fields are passed as extra kwargs, accessible on the LogRecord."""
    result = _run_tracing_script("""
        import logging
        from rattler.rattler import setup_tracing, _emit_test_trace_with_fields

        class ExtraCapture(logging.Handler):
            def emit(self, record):
                extras = {k: getattr(record, k, None) for k in ("a", "b", "c")}
                parts = " ".join(f"{k}={v}" for k, v in extras.items() if v is not None)
                print(f"msg={record.getMessage()!r} {parts}")

        handler = ExtraCapture()
        handler.setLevel(logging.DEBUG)
        logging.root.addHandler(handler)
        logging.root.setLevel(logging.DEBUG)

        setup_tracing("rattler_test=info")
        _emit_test_trace_with_fields("info", "hello", {"x": "1", "y": "2"})
    """)
    # Dict keys sorted: x->a, y->b
    assert _log_lines(result) == [
        "msg='hello' a=1 b=2",
    ]


def test_bridge_message_with_many_fields() -> None:
    """Multiple fields all appear in extra."""
    result = _run_tracing_script("""
        import logging
        from rattler.rattler import setup_tracing, _emit_test_trace_with_fields

        class ExtraCapture(logging.Handler):
            def emit(self, record):
                extras = {k: getattr(record, k, None) for k in ("a", "b", "c", "d")}
                parts = " ".join(f"{k}={v}" for k, v in extras.items() if v is not None)
                print(f"msg={record.getMessage()!r} {parts}")

        handler = ExtraCapture()
        handler.setLevel(logging.DEBUG)
        logging.root.addHandler(handler)
        logging.root.setLevel(logging.DEBUG)

        setup_tracing("rattler_test=info")
        _emit_test_trace_with_fields("info", "event", {"color": "red", "size": "big", "shape": "round", "weight": "heavy"})
    """)
    # Dict keys sorted: color->a, shape->b, size->c, weight->d
    assert _log_lines(result) == [
        "msg='event' a=red b=round c=big d=heavy",
    ]


def test_bridge_fields_only() -> None:
    """No message field -> fields become the message, and are also in extra."""
    result = _run_tracing_script("""
        import logging
        from rattler.rattler import setup_tracing, _emit_test_trace_fields_only

        class ExtraCapture(logging.Handler):
            def emit(self, record):
                extras = {k: getattr(record, k, None) for k in ("a", "b", "c")}
                parts = " ".join(f"{k}={v}" for k, v in extras.items() if v is not None)
                print(f"msg={record.getMessage()!r} {parts}")

        handler = ExtraCapture()
        handler.setLevel(logging.DEBUG)
        logging.root.addHandler(handler)
        logging.root.setLevel(logging.DEBUG)

        setup_tracing("rattler_test=info")
        _emit_test_trace_fields_only("info", {"foo": "1", "bar": "2", "baz": "3"})
    """)
    # Dict keys sorted: bar->a, baz->b, foo->c
    assert _log_lines(result) == [
        "msg='(no message)' a=2 b=3 c=1",
    ]


def test_setup_tracing_reads_rust_log_env() -> None:
    """When no level is passed, setup_tracing should read from RUST_LOG."""
    env = {**os.environ, "RUST_LOG": "rattler_test=info"}
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            textwrap.dedent(
                _LOGGING_PREAMBLE
                + """
        setup_tracing()
        _emit_test_trace("info", "from_rust_log")
        _emit_test_trace("debug", "filtered_out")
        """
            ),
        ],
        capture_output=True,
        text=True,
        timeout=10,
        env=env,
    )
    assert _log_lines(result) == [
        "rattler.rattler_test [INFO] from_rust_log",
    ]


def test_setup_tracing_no_rust_log_defaults_to_off() -> None:
    """When no level is passed and RUST_LOG is unset, nothing should be emitted."""
    env = {k: v for k, v in os.environ.items() if k != "RUST_LOG"}
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            textwrap.dedent(
                _LOGGING_PREAMBLE
                + """
        setup_tracing()
        _emit_test_trace("info", "should_not_appear")
        """
            ),
        ],
        capture_output=True,
        text=True,
        timeout=10,
        env=env,
    )
    assert _log_lines(result) == []


def test_setup_tracing_no_crash_on_exit() -> None:
    """The atexit handler should prevent segfaults during interpreter shutdown."""
    result = _run_tracing_script("""
        import rattler
        rattler.setup_tracing("info")
        print("OK")
    """)
    assert result.returncode == 0, f"returncode: {result.returncode}, stderr: {result.stderr}"
    assert "OK" in result.stdout
