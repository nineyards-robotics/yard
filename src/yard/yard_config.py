from dataclasses import dataclass
from enum import Enum


class RosDistro(Enum):
    HUMBLE = "humble"
    JAZZY = "jazzy"
    KILTED = "kilted"
    ROLLING = "rolling"


@dataclass
class YardConfig:
    ros_distro: RosDistro
