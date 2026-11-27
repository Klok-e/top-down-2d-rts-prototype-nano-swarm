# Settings

The windowed game reads `~/.config/nano-swarm/settings.ron` at launch. If you set `XDG_CONFIG_HOME` to an absolute path, use `$XDG_CONFIG_HOME/nano-swarm/settings.ron`. Quit and relaunch the game to apply changes.

## Presentation mode

```ron
(
    present_mode: mailbox,
)
```

- `mailbox` is the default. It lets the simulation keep running while the window is hidden on supported systems. Bevy chooses another supported mode if mailbox is unavailable.
- `fifo` uses FIFO VSync. On some Wayland systems, the simulation can stall while the window is hidden.

A missing file uses the defaults. If the game cannot read or parse an existing file, it logs a warning and uses the defaults.
