---
status: accepted
---

# The agent socket is always on, and nano-swarm is Unix-only

Every launch of the game binary, including a game the player starts themselves, opens the agent control socket at `$XDG_RUNTIME_DIR/nano-swarm/control.sock`, so an agent can always be asked about the running game. There is no flag to enable or disable it. If the socket cannot be bound, for example because `XDG_RUNTIME_DIR` is unset or another nano-swarm process holds the path, the game refuses to start. The socket needs Unix domain sockets, so non-Unix platforms are no longer supported.

## Considered Options

- **Keep the socket opt-in with `--agent-socket`.** Rejected: the player had to remember the flag before they could ask an agent about a game already in progress.
- **Best effort: warn and run without the socket when binding fails.** Rejected: a game that silently has no socket defeats the point of making it always available.
- **Open the socket only for windowed launches.** Rejected in favour of one rule for every presentation mode.
- **A `--no-agent-socket` opt-out.** Rejected: a second instance can use its own `XDG_RUNTIME_DIR`.
- **Restrict a player-started game's socket to read-only methods.** Rejected: the socket is already private to the user, and a permission split would add a second protocol mode.

## Consequences

- A second instance started with the same `XDG_RUNTIME_DIR` fails with "another nano-swarm control server is active". Agent playtests and verifiers run with a private `XDG_RUNTIME_DIR`, so the standard path stays free for the player's game.
- The binary entry point decides to open the socket. Code that builds the runtime app, such as tests, passes the control configuration explicitly and can omit it.
- Non-Unix builds fail at compile time.
