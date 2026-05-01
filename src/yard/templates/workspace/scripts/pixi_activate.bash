#!/bin/bash
# Make bash-completion discover completions from the pixi/conda environment
# and register ROS 2 argcomplete hooks that live outside the standard path.

# Add conda env to XDG_DATA_DIRS so bash-completion's lazy loader finds
# completions in $CONDA_PREFIX/share/bash-completion/completions/
if [ -n "$CONDA_PREFIX" ]; then
    export XDG_DATA_DIRS="${CONDA_PREFIX}/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

    # Symlink ROS 2 argcomplete scripts into the standard completions dir
    # so they get picked up by bash-completion's on-demand loading.
    _cdir="${CONDA_PREFIX}/share/bash-completion/completions"
    ln -sf "${CONDA_PREFIX}/share/ros2cli/environment/ros2-argcomplete.bash" "$_cdir/ros2" 2>/dev/null
    ln -sf "${CONDA_PREFIX}/share/colcon_argcomplete/hook/colcon-argcomplete.bash" "$_cdir/colcon" 2>/dev/null
    ln -sf "${CONDA_PREFIX}/share/rosidl_cli/environment/rosidl-argcomplete.bash" "$_cdir/rosidl" 2>/dev/null
    unset _cdir
fi
