from __future__ import annotations

from yard.scaffold import Templated, Verbatim

SCAFFOLD = [
    Verbatim(".clang-format"),
    Templated("pixi.toml.j2"),
]
