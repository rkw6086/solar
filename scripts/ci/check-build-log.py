"""Reject actual build diagnostics without matching crate or file names."""
import pathlib
import re
import sys


DIAGNOSTIC = re.compile(
    r"\b(?:warning(?:\[[^\]]+\])?|error(?:\[[^\]]+\])?|"
    r"DeprecationWarning|ExperimentalWarning):|^\s*\(!\)|^\s*(?:Warn|Error)\s",
    re.IGNORECASE,
)
ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def diagnostics(text):
    return [line for line in ANSI_ESCAPE.sub("", text).splitlines() if DIAGNOSTIC.search(line)]


def main():
    found = diagnostics(pathlib.Path(sys.argv[1]).read_text(errors="replace"))
    if found:
        print("Build warnings/errors remain:", file=sys.stderr)
        print("\n".join(found), file=sys.stderr)
        return 1
    print("Clean build completed without warnings or errors.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
