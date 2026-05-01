from __future__ import annotations

from yard.scaffold import Templated, Verbatim

SCAFFOLD = [
    Templated("pixi.toml.j2"),
    Verbatim("pyproject.toml"),
    Verbatim("colcon_defaults.yaml"),
    Verbatim(".clang-format"),
    Verbatim(".cmake-format"),
    Verbatim(".clangd"),
    Verbatim(".mdformat.toml"),
    Verbatim(".gitignore"),
    Verbatim(".pre-commit-config.yaml"),
    Verbatim(".github/workflows/pre-commit.yml"),
    Verbatim(".vscode/settings.json"),
    Verbatim(".vscode/extensions.json"),
    Verbatim(".vscode/tasks.json"),
    Verbatim("scripts/pixi_activate.bash"),
    Verbatim("src/dependencies.repos"),
]
