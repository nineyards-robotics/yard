"""Core module: sets up a ROS 2 workspace with pixi + robostack."""

from __future__ import annotations

from yard.config_files.gitignore import GitIgnore
from yard.config_files.pixi import PixiToml
from yard.module import Context, Module


class RosWorkspaceModule(Module):
    name = "ros_workspace"
    dependencies = []

    def configure(self, ctx: Context) -> None:
        distro = ctx.yard_config.ros_distro

        # -- pixi.toml --
        pixi = ctx.config(PixiToml)  # IDE knows this is PixiToml
        pixi.python_version("3.11")
        pixi.channel("conda-forge")
        pixi.channel(f"robostack-{distro}")
        pixi.dependency(f"ros-{distro}-ros-base")

        # -- .gitignore --
        gitignore = ctx.config(GitIgnore)  # IDE knows this is GitIgnore
        gitignore.block("ros-build", [
            "build/",
            "install/",
            "log/",
        ])
