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

- The container starts a full KDE Plasma Wayland desktop on KWin's virtual
  backend, with Xwayland available for compatibility.
- Sunshine is installed from a pinned release package and launched through
  `arch-sunshine-server`.
- Sunshine should capture through KWin ScreenCast by default, with KMS/auto
  available as explicit configuration choices.
- Sunshine prep hooks resize the KWin virtual desktop from Moonlight client
  width, height, FPS, and scale requests before capture starts.
- Input is provided through Sunshine virtual devices over `/dev/uinput` and
  `/dev/uhid`; the runtime starts udev and seeds `/dev/input/event*` nodes so
  KWin/libinput can discover them.
- Audio is provided globally through PipeWire/Pulse with a virtual sink named
  `arch_sunshine_audio`; Sunshine streams its monitor source.
- Local files are the source of truth. Remote sync should push local changes to
  the remote test host, excluding `.git` and runtime data.

## Verification

Before handing off meaningful runtime changes, at minimum check:

- `python3 -m py_compile build/container/bin/arch-sunshine build/container/bin/arch-sunshine-network-status`
- `bash -n build/container/bin/arch-sunshine-pacman build/container/bin/arch-sunshine-steam build/container/bin/arch-sunshine-steamos-update build/container/bin/arch-sunshine-preseed-steam scripts/remote-sync scripts/remote-clean`
- `python3 -m json.tool config/sunshine/pipelines.json >/dev/null`
- `python3 -m json.tool config/sunshine/apps.json >/dev/null`
- `make dev` if Docker/runtime behavior changed

For GPU/audio fixes, verify from inside the running container:

- `vulkaninfo --summary` sees the GPU.
- `qdbus6 org.kde.KWin /KWin supportInformation` reports OpenGL compositing.
- `arch-sunshine input-test --require-all` verifies that synthetic Linux input
  events reach a Wayland client in the running desktop.
- `pactl info` reports PulseAudio on PipeWire.
- `pactl list short sinks` includes `arch_sunshine_audio`.
- `curl http://127.0.0.1:47989/serverinfo` returns Sunshine server info.
