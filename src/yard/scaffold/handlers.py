from __future__ import annotations

import shutil
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Literal, Protocol

import jinja2

Action = Literal["created", "overwritten", "unchanged", "merged", "skipped"]


@dataclass(frozen=True)
class Context:
    template_dir: Path
    target_dir: Path
    variables: dict[str, Any]
    env: jinja2.Environment


@dataclass(frozen=True)
class FileResult:
    path: Path
    action: Action
    note: str | None = None


@dataclass
class Report:
    results: list[FileResult] = field(default_factory=list)


class Handler(Protocol):
    def apply(self, ctx: Context) -> FileResult: ...


@dataclass(frozen=True)
class Verbatim:
    src: str
    dest: str | None = None

    def _dest(self) -> str:
        return self.dest if self.dest is not None else self.src

    def apply(self, ctx: Context) -> FileResult:
        src_path = ctx.template_dir / self.src
        rel = Path(self._dest())
        dest_path = ctx.target_dir / rel
        dest_path.parent.mkdir(parents=True, exist_ok=True)

        new_bytes = src_path.read_bytes()
        if dest_path.exists() and dest_path.read_bytes() == new_bytes:
            return FileResult(rel, "unchanged")

        action: Action = "overwritten" if dest_path.exists() else "created"
        shutil.copyfile(src_path, dest_path)
        shutil.copymode(src_path, dest_path)
        return FileResult(rel, action)


@dataclass(frozen=True)
class Templated:
    src: str
    dest: str | None = None

    def _dest(self) -> str:
        if self.dest is not None:
            return self.dest
        if self.src.endswith(".j2"):
            return self.src[:-3]
        return self.src

    def apply(self, ctx: Context) -> FileResult:
        template = ctx.env.get_template(self.src)
        rendered = template.render(**ctx.variables)

        rel = Path(self._dest())
        dest_path = ctx.target_dir / rel
        dest_path.parent.mkdir(parents=True, exist_ok=True)

        if dest_path.exists() and dest_path.read_text() == rendered:
            return FileResult(rel, "unchanged")

        action: Action = "overwritten" if dest_path.exists() else "created"
        dest_path.write_text(rendered)
        return FileResult(rel, action)
