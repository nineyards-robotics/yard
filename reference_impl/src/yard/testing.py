"""Test helpers for yard modules and config files.

Provides ``module_context()`` for module tests and nothing else — config file
tests don't need helpers because ``reconcile()`` is already a pure function.

Usage in module tests::

    from yard.testing import module_context

    def test_my_module():
        ctx = module_context(ros_distro="jazzy")
        MyModule().configure(ctx)

        pixi = ctx.config(PixiToml)
        assert pixi.desired_keys == {"dependencies.ros-jazzy-ros-base": "*", ...}

Usage in config file tests::

    def test_gitignore_new_file():
        gi = GitIgnore(Path("/unused"))
        gi._current_source = "test"
        gi.block("build", ["build/", "install/"])

        result = gi.reconcile(existing_content=None)

        assert "build/" in result
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from yard.config_file import ConfigFile
from yard.module import Context, YardConfig, C


class _OpenRegistry:
    """A dict-like object that says 'yes' to any ConfigFile type lookup.

    This means module tests don't need to pre-register ConfigFile types —
    any type passed to ``ctx.config()`` just works.
    """

    def __contains__(self, key: object) -> bool:
        return isinstance(key, type) and issubclass(key, ConfigFile)

    def __getitem__(self, key: type[ConfigFile]) -> type[ConfigFile]:
        return key


def module_context(
    *,
    ros_distro: str = "jazzy",
    module_name: str = "test",
    workspace_root: Path | None = None,
    yard_config_raw: dict[str, Any] | None = None,
) -> Context:
    """Create a ``Context`` for testing modules.

    No entry-point discovery, no disk I/O, no registry setup.
    Any ``ConfigFile`` type is accepted by ``ctx.config()``.

    Parameters
    ----------
    ros_distro:
        Value for ``ctx.yard_config.ros_distro``. Most modules need this.
    module_name:
        Name used as the source in ``DesiredValue`` entries.
    workspace_root:
        Fake workspace root. Defaults to ``Path("/yard-test")``.
        Only matters if you also call ``apply()`` (you normally don't
        in module tests).
    yard_config_raw:
        Full raw dict for ``YardConfig``.  If provided, ``ros_distro``
        is ignored.
    """
    if yard_config_raw is None:
        yard_config_raw = {"workspace": {"ros_distro": ros_distro}}

    yard_config = YardConfig(yard_config_raw)
    root = workspace_root or Path("/yard-test")

    ctx = Context(
        yard_config=yard_config,
        workspace_root=root,
        config_file_registry=_OpenRegistry(),  # type: ignore[arg-type]
    )
    ctx._current_module = module_name
    return ctx
