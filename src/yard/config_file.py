from abc import ABC, abstractmethod
from pathlib import Path


class ConfigFile(ABC):
    _workspace_root: Path

    def __init__(self, workspace_root: Path) -> None:
        self._workspace_root = workspace_root

    @property
    @abstractmethod
    def relative_path(self) -> Path:
        """The path of the config file, relative to workspace root"""
        ...

    @property
    def file_path(self) -> Path:
        return self._workspace_root / self.relative_path

    @abstractmethod
    def reconcile(self, existing_content: str | None) -> str | None:
        """Merge all requested configurations. Error if merge fails. Reconcile existing content with requested
        configurations.
        Optional semantics on input and output:
        input: None = no existing content
        output: None = no file desired (file should be deleted if exists)
        """
        ...

    def apply(self) -> None:
        existing = self.file_path.read_text() if self.file_path.exists() else None
        result = self.reconcile(existing)
        if result is None:
            if self.file_path.exists():
                self.file_path.unlink()
        else:
            self.file_path.parent.mkdir(parents=True, exist_ok=True)
            self.file_path.write_text(result)
