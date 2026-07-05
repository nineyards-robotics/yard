"""Base class for all config file types."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class DesiredValue:
    """A value that a module wants to set in a config file."""

    value: Any
    source: str  # module name that set this


class ConflictError(Exception):
    """Raised when two modules want different values for the same key."""


class ConfigFile(ABC):
    """Base class for typed config file abstractions.

    Subclasses define:
    - ``relative_path``: where the file lives relative to workspace root
    - Typed public methods (the API modules call)
    - ``reconcile()``: pure function that merges desired state with existing
      file content and returns new content

    The public methods accumulate desired state.  The orchestrator calls
    ``apply()`` after all modules have run, which reads from disk, calls
    ``reconcile()``, and writes back.
    """

    relative_path: str  # subclasses set this as a class variable

    def __init__(self, workspace_root: Path) -> None:
        self._workspace_root = workspace_root
        self._desired_keys: dict[str, DesiredValue] = {}
        self._desired_arrays: dict[str, list[DesiredValue]] = {}
        self._current_source: str = "<unknown>"

    @property
    def file_path(self) -> Path:
        return self._workspace_root / self.relative_path

    # -- Public read-only inspection (used in tests) ----------------------

    @property
    def desired_keys(self) -> dict[str, Any]:
        """Accumulated scalar key-values, without source metadata."""
        return {k: v.value for k, v in self._desired_keys.items()}

    @property
    def desired_arrays(self) -> dict[str, list[Any]]:
        """Accumulated array elements, without source metadata."""
        return {
            k: [d.value for d in v] for k, v in self._desired_arrays.items()
        }

    # -- Internal accumulation helpers (called by subclass typed methods) --

    def _set_key(self, key: str, value: Any, *, source: str | None = None) -> None:
        """Record a desired scalar key-value."""
        src = source or self._current_source
        if key in self._desired_keys:
            existing = self._desired_keys[key]
            if existing.value != value:
                raise ConflictError(
                    f"Conflict on '{key}' in {self.relative_path}: "
                    f"module '{existing.source}' wants {existing.value!r}, "
                    f"but module '{src}' wants {value!r}"
                )
            return  # same value, no-op
        self._desired_keys[key] = DesiredValue(value=value, source=src)

    def _append_array(
        self, key: str, value: Any, *, source: str | None = None
    ) -> None:
        """Record a desired array element (additive, no conflicts)."""
        src = source or self._current_source
        arr = self._desired_arrays.setdefault(key, [])
        if not any(d.value == value for d in arr):
            arr.append(DesiredValue(value=value, source=src))

    # -- Reconcile / Apply ------------------------------------------------

    @abstractmethod
    def reconcile(self, existing_content: str | None) -> str:
        """Pure reconciliation: desired state + existing content → new content.

        This is the primary abstraction point for ConfigFile subclasses.
        It takes the current on-disk content (or ``None`` for a new file)
        and returns the full new file content, using the marker system.

        Reads desired state from ``self`` (``_desired_keys``,
        ``_desired_arrays``, and any subclass-specific state).

        Because this is a pure function of (self-state, input-string) →
        output-string, it is trivially testable with no I/O.
        """
        ...

    def apply(self) -> None:
        """Read from disk → reconcile → write back.

        Called by the orchestrator after all modules have run.
        Most subclasses should NOT override this; override ``reconcile()``
        instead.
        """
        existing = (
            self.file_path.read_text() if self.file_path.exists() else None
        )
        result = self.reconcile(existing)
        self.file_path.parent.mkdir(parents=True, exist_ok=True)
        self.file_path.write_text(result)
