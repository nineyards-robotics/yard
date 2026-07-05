from abc import ABC, abstractmethod


class Context:
    pass


class Module(ABC):
    @property
    def name(self) -> str:
        return type(self).__name__

    @abstractmethod
    def configure(self, ctx: Context) -> None:
        pass
