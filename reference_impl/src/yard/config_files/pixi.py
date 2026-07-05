"""Typed abstraction over pixi.toml."""

from __future__ import annotations

from yard.config_file import ConfigFile


class PixiToml(ConfigFile):
    """Programmatic interface to pixi.toml.

    Modules call these methods to express desired state.
    The marker system handles reconciliation with on-disk content.
    """

    relative_path = "pixi.toml"

    def python_version(self, version: str) -> None:
        """Set the Python version constraint."""
        self._set_key("project.requires-python", version)

    def channel(self, name: str) -> None:
        """Add a conda channel."""
        self._append_array("project.channels", name)

    def dependency(self, name: str, spec: str = "*") -> None:
        """Add a conda dependency."""
        self._set_key(f"dependencies.{name}", spec)

    def pypi_dependency(self, name: str, spec: str = "*") -> None:
        """Add a PyPI dependency."""
        self._set_key(f"pypi-dependencies.{name}", spec)

    def task(self, name: str, cmd: str) -> None:
        """Define a pixi task."""
        self._set_key(f"tasks.{name}", cmd)

    def reconcile(self, existing_content: str | None) -> str:
        # TODO: implement TOML read/reconcile/write with marker system
        raise NotImplementedError
