from __future__ import annotations

import importlib.util
from pathlib import Path
from typing import Any

import jinja2
import pytest

from yard import scaffold
from yard.scaffold import Context, Verbatim
from yard.scaffold.render import create_env

FIXTURES_DIR = Path(__file__).parent / "fixtures"
SNAPSHOTS_DIR = Path(__file__).parent / "snapshots"
WORKSPACE_TEMPLATE = Path(__file__).resolve().parents[1] / "src" / "yard" / "templates" / "workspace"
WORKSPACE_VARIABLES = {"workspace_name": "rover", "distro": "jazzy"}


def _load_variables(fixture_dir: Path) -> dict[str, Any]:
    path = fixture_dir / "variables.py"
    if not path.is_file():
        return {}
    spec = importlib.util.spec_from_file_location(f"_yard_test_vars_{fixture_dir.name}", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.VARIABLES


def _files_under(root: Path) -> list[Path]:
    return sorted(p.relative_to(root) for p in root.rglob("*") if p.is_file())


def _assert_tree_matches(actual: Path, expected: Path) -> None:
    actual_files = _files_under(actual)
    expected_files = _files_under(expected)
    assert actual_files == expected_files, (
        f"file lists differ\n  actual: {actual_files}\n  expected: {expected_files}"
    )
    for rel in expected_files:
        actual_text = (actual / rel).read_text()
        expected_text = (expected / rel).read_text()
        assert actual_text == expected_text, f"content mismatch at {rel}"


def _golden_fixtures() -> list[str]:
    if not FIXTURES_DIR.is_dir():
        return []
    return sorted(
        p.name
        for p in FIXTURES_DIR.iterdir()
        if p.is_dir() and (p / "template").is_dir() and (p / "expected").is_dir()
    )


@pytest.mark.parametrize("fixture_name", _golden_fixtures())
def test_fixture_renders_to_expected_tree(tmp_path: Path, fixture_name: str) -> None:
    fixture = FIXTURES_DIR / fixture_name
    variables = _load_variables(fixture)

    scaffold.apply(fixture / "template", tmp_path, variables)

    _assert_tree_matches(tmp_path, fixture / "expected")


def test_verbatim_unchanged_when_target_already_matches(tmp_path: Path) -> None:
    fixture = FIXTURES_DIR / "verbatim_basic"
    expected_text = (fixture / "expected" / ".clang-format").read_text()
    (tmp_path / ".clang-format").write_text(expected_text)

    report = scaffold.apply(fixture / "template", tmp_path, {})

    assert report.results[0].action == "unchanged"


def test_verbatim_overwrites_when_target_differs(tmp_path: Path) -> None:
    fixture = FIXTURES_DIR / "verbatim_basic"
    (tmp_path / ".clang-format").write_text("stale\n")

    report = scaffold.apply(fixture / "template", tmp_path, {})

    assert report.results[0].action == "overwritten"
    expected_text = (fixture / "expected" / ".clang-format").read_text()
    assert (tmp_path / ".clang-format").read_text() == expected_text


def test_templated_strict_undefined_raises(tmp_path: Path) -> None:
    fixture = FIXTURES_DIR / "strict_undefined"

    with pytest.raises(jinja2.UndefinedError):
        scaffold.apply(fixture / "template", tmp_path, {})


def test_handlers_run_in_spec_order(tmp_path: Path) -> None:
    fixture = FIXTURES_DIR / "ordered"

    report = scaffold.apply(fixture / "template", tmp_path, {})

    assert [str(r.path) for r in report.results] == ["c.txt", "a.txt", "b.txt"]


def test_missing_spec_raises(tmp_path: Path) -> None:
    template_dir = tmp_path / "no-spec-here"
    template_dir.mkdir()

    with pytest.raises(FileNotFoundError):
        scaffold.apply(template_dir, tmp_path / "out", {})


def test_workspace_template_matches_snapshot(tmp_path: Path) -> None:
    """Render the real workspace template and diff against tests/snapshots/workspace.

    To regenerate the snapshot after intentional template changes:
        rm -rf tests/snapshots/workspace
        pixi run python -c "from pathlib import Path; from yard.scaffold import apply; \
            d = Path('tests/snapshots/workspace'); d.mkdir(parents=True); \
            apply(Path('src/yard/templates/workspace'), d, \
                  {'workspace_name': 'rover', 'distro': 'jazzy'})"
    """
    scaffold.apply(WORKSPACE_TEMPLATE, tmp_path, WORKSPACE_VARIABLES)
    _assert_tree_matches(tmp_path, SNAPSHOTS_DIR / "workspace")


def test_handler_apply_directly(tmp_path: Path) -> None:
    (tmp_path / "a.txt").write_text("hi\n")
    target = tmp_path / "out"
    target.mkdir()

    ctx = Context(
        template_dir=tmp_path,
        target_dir=target,
        variables={},
        env=create_env(tmp_path),
    )

    result = Verbatim("a.txt").apply(ctx)

    assert result.action == "created"
    assert (target / "a.txt").read_text() == "hi\n"
