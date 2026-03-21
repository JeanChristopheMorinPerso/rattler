from __future__ import annotations

from typing import Optional

from rattler.rattler import setup_tracing as py_setup_tracing


def setup_tracing(directives: Optional[str] = None) -> None:
    """Initialize Rust tracing and bridge events to Python's `logging` module.

    Arguments:
        directives: A tracing filter directive string, e.g. `debug`,
            `info`, or a target-specific filter like
            `rattler_solve=debug`. When `None`, the `RUST_LOG`
            environment variable is used, defaulting to `off` if unset.
    """
    py_setup_tracing(directives=directives)


__all__ = ["setup_tracing"]
