"""Tests for ConfigFile reconciliation.

ConfigFile tests exercise the pure ``reconcile()`` method:
  - Set desired state via the typed API
  - Call ``reconcile(existing_content)``
  - Assert on the output string

No disk I/O, no tmp dirs.  String in → string out.
"""

from pathlib import Path

import pytest

from yard.config_files.gitignore import GitIgnore
from yard.config_files.pixi import PixiToml


# -- GitIgnore -------------------------------------------------------------


class TestGitIgnoreAccumulation:
    """Test the typed API and state accumulation (pre-reconcile)."""

    def _make(self) -> GitIgnore:
        gi = GitIgnore(Path("/unused"))
        gi._current_source = "test"
        return gi

    def test_single_block(self):
        gi = self._make()
        gi.block("build", ["build/", "install/", "log/"])

        assert gi.desired_blocks == {
            "build": ["build/", "install/", "log/"],
        }

    def test_merge_same_block_id(self):
        gi = self._make()
        gi.block("build", ["build/", "install/"])
        gi.block("build", ["log/", "build/"])  # "build/" deduped

        assert gi.desired_blocks == {
            "build": ["build/", "install/", "log/"],
        }

    def test_multiple_block_ids(self):
        gi = self._make()
        gi.block("build", ["build/"])
        gi.block("env", [".venv/"])

        assert gi.desired_blocks == {
            "build": ["build/"],
            "env": [".venv/"],
        }


class TestGitIgnoreReconcile:
    """Test the pure reconcile() method — string in, string out."""

    @pytest.mark.skip(reason="reconcile() not yet implemented")
    def test_new_file(self):
        gi = GitIgnore(Path("/unused"))
        gi._current_source = "test"
        gi.block("build", ["build/", "install/"])

        result = gi.reconcile(existing_content=None)

        assert result == (
            "# >>> yard:managed id=build >>>\n"
            "build/\n"
            "install/\n"
            "# <<< yard:managed id=build <<<\n"
        )

    @pytest.mark.skip(reason="reconcile() not yet implemented")
    def test_preserves_user_content(self):
        existing = (
            "# My custom ignores\n"
            "*.swp\n"
            "\n"
            "# >>> yard:managed id=build >>>\n"
            "build/\n"
            "install/\n"
            "# <<< yard:managed id=build <<<\n"
        )

        gi = GitIgnore(Path("/unused"))
        gi._current_source = "test"
        gi.block("build", ["build/", "install/", "log/"])

        result = gi.reconcile(existing_content=existing)

        # User content preserved, managed block updated
        assert "*.swp" in result
        assert "log/" in result


# -- PixiToml --------------------------------------------------------------


class TestPixiTomlAccumulation:
    """Test the typed API and state accumulation (pre-reconcile)."""

    def _make(self) -> PixiToml:
        p = PixiToml(Path("/unused"))
        p._current_source = "test"
        return p

    def test_python_version(self):
        p = self._make()
        p.python_version("3.11")

        assert p.desired_keys == {"project.requires-python": "3.11"}

    def test_channels(self):
        p = self._make()
        p.channel("conda-forge")
        p.channel("robostack-jazzy")

        assert p.desired_arrays == {
            "project.channels": ["conda-forge", "robostack-jazzy"],
        }

    def test_dependencies(self):
        p = self._make()
        p.dependency("numpy", ">=1.24")
        p.dependency("ros-jazzy-ros-base")

        assert p.desired_keys == {
            "dependencies.numpy": ">=1.24",
            "dependencies.ros-jazzy-ros-base": "*",
        }

    def test_full_workspace_config(self):
        """A realistic combination of settings."""
        p = self._make()
        p.python_version("3.11")
        p.channel("conda-forge")
        p.channel("robostack-jazzy")
        p.dependency("ros-jazzy-ros-base")
        p.task("build", "colcon build")

        assert p.desired_keys == {
            "project.requires-python": "3.11",
            "dependencies.ros-jazzy-ros-base": "*",
            "tasks.build": "colcon build",
        }
        assert p.desired_arrays == {
            "project.channels": ["conda-forge", "robostack-jazzy"],
        }
