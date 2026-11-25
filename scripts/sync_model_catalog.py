"""Refresh the Desktop Nova model presets from the sibling CLI checkout."""

import ast
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checkout = Path(sys.argv[1]) if len(sys.argv) > 1 else root.parent / "NovaCode"
source = checkout / "novacode_cli/config/model_manager.py"
tree = ast.parse(source.read_text(encoding="utf-8"))
presets = next(
    node.value for node in tree.body
    if isinstance(node, ast.AnnAssign)
    and isinstance(node.target, ast.Name)
    and node.target.id == "MODEL_PRESETS"
)
constants = {
    node.targets[0].id: node.value.value
    for node in tree.body
    if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)
    and isinstance(node.value, ast.Constant) and isinstance(node.value.value, str)
}
models = []
for provider, preset in zip(presets.keys, presets.values, strict=True):
    for key, value in zip(preset.keys, preset.values, strict=True):
        if isinstance(key, ast.Constant) and key.value == "models" and isinstance(value, (ast.List, ast.Tuple)):
            for entry in value.elts:
                model = entry.value if isinstance(entry, ast.Constant) else constants.get(entry.id) if isinstance(entry, ast.Name) else None
                if isinstance(model, str):
                    models.append(f"{provider.value}:{model}")
snapshot = json.loads(source.with_name("provider_catalog.json").read_text(encoding="utf-8"))
models.extend(f"{provider}:{model}" for provider, preset in snapshot["providers"].items() for model in preset.get("models", []))
models = list(dict.fromkeys(models))
assert models and all(model.count(":") and not model.endswith(":") for model in models)
(root / "src/lib/nova-models.json").write_text(json.dumps(models, indent=2) + "\n", encoding="utf-8")
