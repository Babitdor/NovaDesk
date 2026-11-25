"""Refresh the Desktop slash catalog from the sibling NovaCode checkout."""
import ast
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
checkout = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT.parent / "NovaCode"
nova = checkout / "novacode_cli"


def read_ast(path: Path) -> ast.Module:
    return ast.parse(path.read_text(encoding="utf-8"), filename=str(path))


def function(tree: ast.Module, name: str) -> ast.FunctionDef | ast.AsyncFunctionDef:
    return next(node for node in tree.body
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name)


table = next(node.value for node in read_ast(nova / "tui/app.py").body
             if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)
             and node.target.id == "TUI_COMMANDS")
descriptions = {}
aliases = {}
for key, call in zip(table.keys, table.values, strict=True):
    name = ast.literal_eval(key)
    descriptions[name] = ast.literal_eval(call.args[1])
    for keyword in call.keywords:
        if keyword.arg == "aliases":
            aliases.update((alias, name) for alias in ast.literal_eval(keyword.value))

registry = function(read_ast(nova / "commands/__init__.py"), "build_command_registry")
pipe_commands = set()
for node in ast.walk(registry):
    if not isinstance(node, ast.ImportFrom) or not node.module:
        continue
    if not any(alias.name == "register_commands" for alias in node.names):
        continue
    module = nova.parent.joinpath(*node.module.split(".")).with_suffix(".py")
    registration = function(read_ast(module), "register_commands")
    for call in ast.walk(registration):
        if (isinstance(call, ast.Call) and isinstance(call.func, ast.Attribute)
                and call.func.attr == "register" and call.args and isinstance(call.args[0], ast.Constant)):
            pipe_commands.add(ast.literal_eval(call.args[0]))

dispatch = function(read_ast(nova / "commands/commands.py"), "handle_command")
for node in ast.walk(dispatch):
    if not isinstance(node, ast.If) or not isinstance(node.test, ast.Compare):
        continue
    test = node.test
    if not isinstance(test.left, ast.Name) or test.left.id != "cmd" or len(test.ops) != 1:
        continue
    if isinstance(test.ops[0], (ast.Eq, ast.In)):
        target = ast.literal_eval(test.comparators[0])
        pipe_commands.update(target if isinstance(target, (tuple, list, set)) else (target,))

descriptions["skill:"] = "Invoke a Nova skill by name"
pipe_commands.add("skill:")
interactive = {"agents", "auth", "connect", "hooks", "middleware", "model", "router",
               "sessions", "skills", "trace", "quit", "exit", "q"}
rows = []
for name in sorted(pipe_commands | descriptions.keys() | aliases.keys()):
    canonical = aliases.get(name, name)
    description = descriptions.get(canonical, "Run this Nova command")
    surface = "chat" if canonical in pipe_commands and canonical not in interactive else "terminal"
    rows.append([name, canonical, description, surface])

target = ROOT / "src/lib/nova-commands.json"
target.write_text("[\n" + ",\n".join("  " + json.dumps(row, ensure_ascii=False) for row in rows)
                  + "\n]\n", encoding="utf-8")
print(f"{len(rows)} Nova commands and aliases")
