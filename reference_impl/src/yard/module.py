"""Base class for yard modules and the Context they receive."""

from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any, TypeVar, overload

from yard.config_file import ConfigFile

C = TypeVar("C", bound=ConfigFile)


class YardConfig:
    """Parsed read-only view of yard.toml."""

    def __init__(self, raw: dict[str, Any]) -> None:
        self._raw = raw

    @property
    def ros_distro(self) -> str:
        return self._raw["workspace"]["ros_distro"]

    def module_config(self, module_name: str) -> dict[str, Any]:
        """Get the [modules.<name>] config section, or empty dict."""
        return self._raw.get("modules", {}).get(module_name, {})


class Context:
    """Passed to each module's configure(). Provides typed access to ConfigFiles."""

    def __init__(
        self,
        yard_config: YardConfig,
        workspace_root: "Path",  # noqa: F821
        config_file_registry: dict[type[ConfigFile], type[ConfigFile]],
    ) -> None:
        self.yard_config = yard_config
        self._workspace_root = workspace_root
        self._registry = config_file_registry
        self._instances: dict[type[ConfigFile], ConfigFile] = {}
        self._current_module: str = "<unknown>"

    def config(self, cls: type[C]) -> C:
        """Get (or lazily create) a ConfigFile instance by its type.

        Returns the exact type you asked for — full autocomplete and type checking.

        Raises KeyError if the ConfigFile type isn't registered (i.e. the
        package providing it isn't installed).
        """
        if cls not in self._instances:
            if cls not in self._registry:
                raise KeyError(
                    f"ConfigFile type {cls.__name__} is not registered. "
                    f"Is the package providing it installed?"
                )
            instance = cls(self._workspace_root)
            self._instances[cls] = instance
        inst = self._instances[cls]
        inst._current_source = self._current_module
        return inst  # type: ignore[return-value]

    def config_optional(self, cls: type[C]) -> C | None:
        """Like config(), but returns None if the type isn't registered.

        Use this for optional cross-extension interactions:

            colcon = ctx.config_optional(ColconConfig)
            if colcon is not None:
                colcon.defaults_file("colcon.yaml")
        """
        if cls not in self._registry:
            return None
        return self.config(cls)

    def instantiated_configs(self) -> list[ConfigFile]:
        """All ConfigFile instances that were actually requested by modules."""
        return list(self._instances.values())


class Module(ABC):
    """Base class for yard modules.

    Subclasses implement configure() to express their desired config state
    by calling typed methods on ConfigFile instances obtained via ctx.config().
    """

    # Module name — used for conflict reporting and yard.toml section lookup
    name: str

    # Other modules that must run before this one
    dependencies: list[str] = []

    @abstractmethod
    def configure(self, ctx: Context) -> None:
        """Express desired configuration state.

        Call ctx.config(SomeConfigType) to get typed config file instances,
        then call their methods to accumulate desired state.
        """
        ...
