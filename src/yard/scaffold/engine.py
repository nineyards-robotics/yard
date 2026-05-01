from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType
from typing import Any

from yard.scaffold.handlers import Context, Handler, Report
from yard.scaffold.render import create_env


def apply(template_dir: Path, target_dir: Path, variables: dict[str, Any]) -> Report:
    template_dir = template_dir.resolve()
    target_dir = target_dir.resolve()
    target_dir.mkdir(parents=True, exist_ok=True)

    spec_module = _load_spec(template_dir / "_spec.py")
    handlers: list[Handler] = list(spec_module.SCAFFOLD)

    ctx = Context(
        template_dir=template_dir,
        target_dir=target_dir,
        variables=variables,
        env=create_env(template_dir),
    )

    report = Report()
    for handler in handlers:
        report.results.append(handler.apply(ctx))
    return report


def _load_spec(spec_path: Path) -> ModuleType:
    if not spec_path.is_file():
        raise FileNotFoundError(f"template spec not found: {spec_path}")
    spec = importlib.util.spec_from_file_location(f"_yard_spec_{spec_path.parent.name}", spec_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"could not load spec module at {spec_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if not hasattr(module, "SCAFFOLD"):
        raise AttributeError(f"{spec_path} is missing SCAFFOLD")
    return module
