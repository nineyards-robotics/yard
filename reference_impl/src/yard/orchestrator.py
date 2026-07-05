"""Orchestrator: discovers plugins, runs modules, applies config files."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

from yard.config_file import ConfigFile
from yard.module import Context, Module, YardConfig

if sys.version_info >= (3, 12):
    from importlib.metadata import entry_points
else:
    from importlib.metadata import entry_points


def _discover_entry_points(group: str) -> dict[str, Any]:
    """Load all entry points for a given group."""
    eps = entry_points(group=group)
    return {ep.name: ep.load() for ep in eps}


def _topo_sort(modules: dict[str, type[Module]]) -> list[str]:
    """Topological sort of modules by their declared dependencies."""
    visited: set[str] = set()
    order: list[str] = []

    def visit(name: str) -> None:
        if name in visited:
            return
        visited.add(name)
        cls = modules[name]
        for dep in cls.dependencies:
            if dep not in modules:
                raise ValueError(
                    f"Module '{name}' depends on '{dep}', which is not registered."
                )
            visit(dep)
        order.append(name)

    for name in modules:
        visit(name)
    return order


def run(workspace_root: Path, yard_config_raw: dict[str, Any]) -> None:
    """Main entry point: discover, configure, apply."""
    yard_config = YardConfig(yard_config_raw)

    # 1. Discover all registered ConfigFile and Module types
    config_file_types: dict[str, type[ConfigFile]] = _discover_entry_points(
        "yard.config_files"
    )
    module_types: dict[str, type[Module]] = _discover_entry_points("yard.modules")

    # 2. Build the type registry (type → type, for ctx.config() lookup)
    type_registry: dict[type[ConfigFile], type[ConfigFile]] = {
        cls: cls for cls in config_file_types.values()
    }

    # 3. Filter to enabled modules
    modules_config = yard_config_raw.get("modules", {})
    enabled_modules: dict[str, type[Module]] = {}
    for name, cls in module_types.items():
        mod_cfg = modules_config.get(name, {})
        # A module is enabled if: explicitly enabled, or present without enabled=false
        if isinstance(mod_cfg, bool):
            if mod_cfg:
                enabled_modules[name] = cls
        elif isinstance(mod_cfg, dict):
            if mod_cfg.get("enabled", True):
                enabled_modules[name] = cls

    # 4. Topological sort
    ordered = _topo_sort(enabled_modules)

    # 5. Create context and run modules
    ctx = Context(yard_config, workspace_root, type_registry)

    for name in ordered:
        cls = enabled_modules[name]
        module = cls()
        ctx._current_module = name
        module.configure(ctx)

    # 6. Apply all instantiated config files
    for config_file in ctx.instantiated_configs():
        config_file.apply()
