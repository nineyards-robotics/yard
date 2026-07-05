"""Typed abstraction over .gitignore."""

from __future__ import annotations

from pathlib import Path

from yard.config_file import ConfigFile


class GitIgnore(ConfigFile):
    """Programmatic interface to .gitignore using block markers.

    Each block gets a unique id and is fenced with yard:managed markers.
    """

    relative_path = ".gitignore"

    def __init__(self, workspace_root: Path) -> None:
        super().__init__(workspace_root)
        self._desired_blocks: dict[str, list[str]] = {}

    def block(self, block_id: str, patterns: list[str]) -> None:
        """Add a managed block of .gitignore patterns.

        Multiple calls with the same block_id merge patterns.
        """
        existing = self._desired_blocks.setdefault(block_id, [])
        for p in patterns:
            if p not in existing:
                existing.append(p)

    # -- Public read-only inspection (used in tests) ----------------------

    @property
    def desired_blocks(self) -> dict[str, list[str]]:
        """Accumulated block contents, keyed by block id."""
        return {k: list(v) for k, v in self._desired_blocks.items()}

    def reconcile(self, existing_content: str | None) -> str:
        # TODO: implement block-marker read/reconcile/write
        raise NotImplementedError
