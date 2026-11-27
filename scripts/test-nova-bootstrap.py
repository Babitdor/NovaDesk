"""Exercise the embedded compatibility function without starting a model."""
import ast
import json
import unittest
from pathlib import Path
from types import SimpleNamespace


class DesktopArguments(unittest.TestCase):
    def setUp(self):
        tree = ast.parse((Path(__file__).parents[1] / "src-tauri/src/agent/personality.py").read_text(encoding="utf-8"))
        function = next(node for node in ast.walk(tree) if isinstance(node, ast.FunctionDef) and node.name == "desktop_arguments")
        self.validations = []

        def validate(arguments):
            self.validations.append(arguments)
            if not isinstance(arguments, dict) or not isinstance(arguments.get("title"), str):
                raise ValueError("Invalid title")

        self.repair = lambda *_: None
        namespace = {
            "json": json,
            "code_mode": SimpleNamespace(_validator=lambda *_, **__: SimpleNamespace(validate=validate)),
            "repair_dispatch_args": lambda *args: self.repair(*args),
        }
        exec(compile(ast.Module(body=[function], type_ignores=[]), "desktop_arguments", "exec"), namespace)
        self.call = namespace["desktop_arguments"]

    def test_valid_arguments_are_retained_without_losing_validation(self):
        arguments = {"title": "Fixture"}
        self.assertIs(self.call(None, SimpleNamespace(name="nova-desktop_create_task"), arguments, {}, {}), arguments)
        self.assertEqual(self.validations, [arguments])

    def test_invalid_arguments_are_rejected(self):
        with self.assertRaises(ValueError):
            self.call(None, SimpleNamespace(name="nova-desktop_create_task"), None, {}, {})

    def test_upstream_repairs_and_unrelated_tools_keep_their_behavior(self):
        repaired = {"title": "Repaired"}
        self.repair = lambda *_: repaired
        self.assertIs(self.call(None, SimpleNamespace(name="nova-desktop_create_task"), {}, {}, {}), repaired)
        self.repair = lambda *_: None
        self.assertIsNone(self.call(None, SimpleNamespace(name="other_tool"), {}, {}, {}))
        self.assertEqual(self.validations, [])


if __name__ == "__main__":
    unittest.main()
