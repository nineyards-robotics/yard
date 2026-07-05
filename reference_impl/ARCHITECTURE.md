# Yard Architecture

## Overview

Yard has a two-stage plugin architecture:

1. **ConfigFiles** — typed abstractions over specific config file formats (pixi.toml, .gitignore, colcon.yaml). They accumulate "desired state" from modules, then reconcile with on-disk state using the marker system.

2. **Modules** — encapsulate a workspace concern (e.g., `ros_workspace`, `python_tooling`). They read from `YardConfig` and call typed methods on ConfigFile instances.

Both are extensible via Python entry points.

## Core Design: Modules talk to ConfigFiles by type

```python
class RosWorkspaceModule(Module):
    def configure(self, ctx: Context) -> None:
        pixi = ctx.config(PixiToml)            # returns PixiToml, fully typed
        gitignore = ctx.config(GitIgnore)       # returns GitIgnore, fully typed

        distro = ctx.yard_config.ros_distro     # from yard.toml

        pixi.python_version("3.11")
        pixi.channel(f"robostack-{distro}")
        pixi.dependency(f"ros-{distro}-desktop")

        gitignore.block("ros-build", ["build/", "install/", "log/"])
```

`ctx.config(T)` is generic — the return type IS `T`. Full autocomplete, full type checking.

## How `ctx.config()` works

The `Context` holds a **lazy registry** of ConfigFile instances, keyed by type:

```python
class Context:
    def config(self, cls: type[C]) -> C:
        if cls not in self._instances:
            self._instances[cls] = cls(self._workspace_root)
        return self._instances[cls]
```

ConfigFile types are discovered via entry points at startup, so the Context
knows which types are available. But instances are only created when a module
actually asks for one. If no module touches `ColconConfig`, it never gets
instantiated and its file is never written.

## ConfigFile: accumulator + reconciler

ConfigFile methods don't write to disk immediately. They record desired state:

```python
class PixiToml(ConfigFile):
    path = "pixi.toml"

    def __init__(self, workspace_root: Path):
        super().__init__(workspace_root)
        self._desired: dict[str, DesiredValue] = {}

    def python_version(self, version: str) -> None:
        self._set_key("project.python", version)

    def channel(self, name: str) -> None:
        self._append_array("project.channels", name)

    def dependency(self, name: str, spec: str = "*") -> None:
        self._set_key(f"dependencies.{name}", spec)
```

After all modules have run, the orchestrator calls `config_file.apply()` on
each instantiated ConfigFile. That's where marker-system reconciliation
happens: compare desired state vs on-disk state, detect conflicts, write
updates.

## Module ↔ ConfigFile coupling: why it's fine

A module that calls `ctx.config(PixiToml)` must be able to `import PixiToml`.
This means:

- **For core config files** (pixi.toml, .gitignore, colcon.yaml): they ship
  with yard. Every module can import them. Zero friction.

- **For third-party config files**: the module's package declares a Python
  dependency on the package that defines the ConfigFile. This is explicit,
  pip-resolvable, and the type checker sees everything.

- **For optional interaction**: a module can use `ctx.config_optional(SomeType)`
  which returns `SomeType | None`. The type is only available if the package
  is installed. This handles "if someone has installed the X extension, I'd
  like to configure it too."

This coupling is *intentional*. It's the same pattern as importing a library
to use its API. The alternative (stringly-typed intents) gives you decoupling
at the cost of all type safety — bad trade.

## Discovery via entry points

Both ConfigFiles and Modules register via standard Python entry points:

```toml
# yard's own pyproject.toml
[project.entry-points."yard.config_files"]
pixi_toml = "yard.config_files.pixi:PixiToml"
gitignore = "yard.config_files.gitignore:GitIgnore"
colcon = "yard.config_files.colcon:ColconConfig"

[project.entry-points."yard.modules"]
ros_workspace = "yard.modules.ros_workspace:RosWorkspaceModule"
```

```toml
# third-party extension's pyproject.toml
[project.entry-points."yard.config_files"]
my_config = "my_yard_ext.configs:MyCustomConfig"

[project.entry-points."yard.modules"]
my_module = "my_yard_ext.modules:MyModule"
```

At startup, yard loads all entry points to build the type registry.

## yard.toml

The user-facing config. Read-only input to modules.

```toml
[workspace]
ros_distro = "jazzy"

[modules.ros_workspace]
enabled = true

[modules.my_custom_module]
enabled = true
extra_packages = ["nav2", "moveit"]
```

Each module gets its own config section. The module reads it via
`ctx.yard_config.module_config("ros_workspace")` or similar typed accessor.

## Execution flow

```
1.  Parse yard.toml → YardConfig
2.  Discover all ConfigFile types via entry points → type registry
3.  Discover all Module types via entry points
4.  Filter to enabled modules (from yard.toml)
5.  Create Context (with type registry, YardConfig, workspace root)
6.  For each enabled module (in dependency order):
        module.configure(ctx)
        # module calls ctx.config(T) to get typed ConfigFile instances
        # module calls methods like pixi.dependency("foo")
        # ConfigFile accumulates desired state internally
7.  For each instantiated ConfigFile:
        config_file.apply()
        # reads on-disk file
        # reconciles desired state vs on-disk via marker system
        # writes updated file (or reports conflicts)
```

## Conflict between modules

Two modules might both call `pixi.python_version()` with different values.
This is detected at accumulation time inside the ConfigFile:

```python
def _set_key(self, key: str, value: Any) -> None:
    if key in self._desired and self._desired[key].value != value:
        raise ConflictError(
            f"Conflict on {key}: "
            f"{self._desired[key].source} wants {self._desired[key].value!r}, "
            f"but got {value!r}"
        )
    self._desired[key] = DesiredValue(value=value, source=self._current_module)
```

For additive operations (channels, dependencies, gitignore blocks), there's
no conflict — they merge naturally.

## Module ordering and dependencies

Modules can declare dependencies on other modules:

```python
class MyModule(Module):
    dependencies = ["ros_workspace"]
```

The orchestrator topologically sorts modules so dependencies run first.
This matters when module B wants to read/react to what module A configured
(though in most cases modules are independent and just accumulate into
ConfigFiles).
