from __future__ import annotations

import typer

app = typer.Typer(help="yard - Batteries-included ROS2 workspaces", no_args_is_help=True)


@app.callback()
def main() -> None:
    """yard - Batteries-included ROS2 workspaces"""


@app.command()
def hello(name: str = "world") -> None:
    """Print a greeting."""
    typer.echo(f"hello, {name}")


if __name__ == "__main__":
    app()
