"""Tests for yard modules.

Module tests are pure state-accumulation checks — no disk I/O, no mocks.
Each test:
  1. Creates a test context with the relevant yard.toml config
  2. Runs the module
  3. Asserts on the desired state accumulated in each ConfigFile
"""

from yard.config_file import ConflictError
from yard.config_files.gitignore import GitIgnore
from yard.config_files.pixi import PixiToml
from yard.modules.ros_workspace import RosWorkspaceModule
from yard.testing import module_context

import pytest


# -- RosWorkspaceModule ----------------------------------------------------


class TestRosWorkspaceModule:
    def test_jazzy(self):
        ctx = module_context(ros_distro="jazzy")
        RosWorkspaceModule().configure(ctx)

        pixi = ctx.config(PixiToml)
        assert pixi.desired_keys == {
            "project.requires-python": "3.11",
            "dependencies.ros-jazzy-ros-base": "*",
        }
        assert pixi.desired_arrays == {
            "project.channels": ["conda-forge", "robostack-jazzy"],
        }

        gitignore = ctx.config(GitIgnore)
        assert gitignore.desired_blocks == {
            "ros-build": ["build/", "install/", "log/"],
        }

    def test_humble(self):
        ctx = module_context(ros_distro="humble")
        RosWorkspaceModule().configure(ctx)

        pixi = ctx.config(PixiToml)
        assert pixi.desired_arrays == {
            "project.channels": ["conda-forge", "robostack-humble"],
        }
        assert "dependencies.ros-humble-ros-base" in pixi.desired_keys

    def test_only_touches_expected_config_files(self):
        ctx = module_context(ros_distro="jazzy")
        RosWorkspaceModule().configure(ctx)

        instantiated = {type(c).__name__ for c in ctx.instantiated_configs()}
        assert instantiated == {"PixiToml", "GitIgnore"}


# -- Cross-module conflict detection --------------------------------------


class TestModuleConflicts:
    def test_conflicting_python_version(self):
        ctx = module_context(ros_distro="jazzy")

        # Module A sets python 3.11
        ctx._current_module = "module_a"
        pixi = ctx.config(PixiToml)
        pixi.python_version("3.11")

        # Module B tries python 3.12 → conflict
        ctx._current_module = "module_b"
        pixi = ctx.config(PixiToml)
        with pytest.raises(ConflictError, match="module_a.*module_b"):
            pixi.python_version("3.12")

    def test_same_value_no_conflict(self):
        ctx = module_context(ros_distro="jazzy")

        ctx._current_module = "module_a"
        ctx.config(PixiToml).python_version("3.11")

        ctx._current_module = "module_b"
        ctx.config(PixiToml).python_version("3.11")  # same value, no error

    def test_additive_arrays_never_conflict(self):
        ctx = module_context(ros_distro="jazzy")

        ctx._current_module = "module_a"
        ctx.config(PixiToml).channel("conda-forge")

        ctx._current_module = "module_b"
        ctx.config(PixiToml).channel("pytorch")

        assert ctx.config(PixiToml).desired_arrays == {
            "project.channels": ["conda-forge", "pytorch"],
        }

    def test_duplicate_array_values_deduped(self):
        ctx = module_context(ros_distro="jazzy")

        ctx._current_module = "module_a"
        ctx.config(PixiToml).channel("conda-forge")

        ctx._current_module = "module_b"
        ctx.config(PixiToml).channel("conda-forge")

        assert ctx.config(PixiToml).desired_arrays == {
            "project.channels": ["conda-forge"],
        }
