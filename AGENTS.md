# Agent Notes

This project is a barebones Arch Linux desktop container for Sunshine,
Moonlight, KDE Plasma, Steam, and Firefox.

## Working Rules

- Do not commit unless the user explicitly asks for a commit.
- Keep the project simple: no extra env files, test framework, or broad tooling
  unless the user asks for it.
- Prefer global container/session fixes over app-specific launch wrappers.
  Steam, games, Firefox, and desktop tools should inherit one working desktop
  environment.
- Do not add per-game workarounds to the default path. If a game needs a special
  setting, document it separately and make it opt-in.
- Do not store secrets, host passwords, or private machine-specific details in
  the repo.
- Preserve the single persistent mount model: user data should live under
  `/mnt/user_data`, backed by `./mnt/user_data` from Compose.

## Commands

- `make dev` builds, starts, and attaches to the container UI.
- `make clean` stops the container and removes local persisted data.
- `make remote-dev` syncs local files to the remote test host, rebuilds there,
  and attaches.
- `make remote-watch` keeps local edits mirrored to the remote test host.

## Runtime Shape

- The container starts KDE Plasma on a GPU-backed X11 display when possible.
- Headless GPU mode uses Weston headless plus rootful Xwayland for X11 capture.
- Sunshine captures the X11 desktop and should auto-select the best available
  encoder.
- Audio is provided globally through PipeWire/Pulse with a virtual sink named
  `arch_sunshine_audio`; Sunshine streams its monitor source.
- Local files are the source of truth. Remote sync should push local changes to
  the remote test host, excluding `.git` and runtime data.

## Verification

Before handing off meaningful runtime changes, at minimum check:

- `bash -n build/container/bin/arch-sunshine`
- `python3 -m py_compile build/container/bin/arch-sunshine-input-bridge`
- `make dev` if Docker/runtime behavior changed

For GPU/audio fixes, verify from inside the running container:

- `glxinfo -B` shows accelerated rendering on the mounted GPU.
- `vulkaninfo --summary` sees the GPU.
- `pactl info` reports PulseAudio on PipeWire.
- `pactl list short sinks` includes `arch_sunshine_audio`.
