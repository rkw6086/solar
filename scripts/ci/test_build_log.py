import importlib.util
import pathlib
import unittest


spec = importlib.util.spec_from_file_location("build_log", pathlib.Path(__file__).with_name("check-build-log.py"))
build_log = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build_log)


class BuildLogTests(unittest.TestCase):
    def test_compiler_and_bundler_diagnostics_are_rejected(self):
        lines = [
            "warning: unused import: Thing",
            "warning[E123]: fixture diagnostic",
            "source.swift:12:3: warning: deprecated API",
            "error[E0599]: method does not exist",
            "(node:123) [DEP0005] DeprecationWarning: Buffer() is deprecated",
            "(node:123) ExperimentalWarning: experimental API",
            "(!) Some chunks are larger than 500 kB after minification.",
            "        Warn --no-sign flag detected: Signing will be skipped.",
            "       Error failed to build application",
            "\x1b[33mwarning:\x1b[0m unused variable",
        ]
        self.assertEqual(len(build_log.diagnostics("\n".join(lines))), len(lines))

    def test_ordinary_crate_and_path_names_are_not_diagnostics(self):
        self.assertEqual(build_log.diagnostics("\n".join([
            "Compiling thiserror v2.0.17",
            "Compiling warnings v0.1.0",
            "Running tests/error_handling.rs",
            "Finished release profile [optimized] target(s) in 20s",
        ])), [])


if __name__ == "__main__":
    unittest.main()
