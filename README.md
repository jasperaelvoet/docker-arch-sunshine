# Docker Arch Sunshine

Arch Linux desktop container for Moonlight game streaming through Sunshine. It
runs KDE Plasma with Steam and Firefox, persists user data locally, and is set up
for GPU-backed desktop capture on headless Linux hosts.

## What's included

- Arch Linux desktop with KDE Plasma
- Sunshine host for Moonlight pairing and streaming
- Steam and Firefox launchers
- PipeWire/PulseAudio audio capture for streams
- Persistent desktop, Steam, and Sunshine state in `./mnt/user_data`
- Stream-triggered desktop wake/sleep so idle containers keep only Sunshine and
  a lightweight X11 capture display up
- Plasma Disconnect launcher for closing the Moonlight stream and sleeping the
  desktop session

## Requirements

- Docker with the Compose plugin
- Linux host with GPU devices exposed at `/dev/dri`
- `/dev/uinput` and `/dev/uhid` available for input/controller passthrough
- Host networking available for Sunshine and Moonlight discovery

## Usage

```sh
make dev
```

This builds the image, starts the container, and attaches to the terminal control
UI. Pair from Moonlight, then press `p` in the control UI to enter the pairing
PIN.

## Published Image

Every push to `main` publishes the container to GitHub Container Registry:

```sh
docker pull ghcr.io/jasperaelvoet/docker-arch-sunshine:latest
```

## Commands

```sh
make dev    # build, start, and attach
make clean  # stop and remove persisted local data
```

## Remote Host

The remote host is treated as a test machine. Local files stay the source of
truth.

```sh
make remote-dev      # sync, rebuild on remote, and attach
make remote-sync     # sync files only
make remote-watch    # keep syncing local edits to remote
make remote-clean    # stop remote container and remove remote persisted data
```

Override the target if needed:

```sh
make remote-dev REMOTE=root@10.10.10.122 REMOTE_DIR=/root/docker-games
```

## Defaults

- Image/container: `docker-arch-sunshine`
- Desktop user: `sunshine`
- Desktop password: `sunshine`
- Sunshine Web UI: `https://<host-ip>:47990`
- Web UI login: `sunshine` / `sunshine`
- Persistent data: `./mnt/user_data`

Runtime system changes are intentionally blocked. The container creates its own
ephemeral writable runtime mounts, then remounts the root filesystem read-only
when the Docker host supports it. On hosts that reject root overlay remounts,
the entrypoint falls back to read-only system path mounts for package and OS
state. User/session data stays under `/mnt/user_data`. Add or remove system
packages in `build/container/Dockerfile`, then rebuild the image.
Steam is seeded from the packaged bootstrap into persistent user data on first
start; client updates and shader cache state stay under `/mnt/user_data`.

The container uses the mounted GPU when available. On a headless GPU it runs
KDE Plasma on GPU-backed Xwayland so Sunshine can still capture an X11 desktop.
The lightweight X11 capture display stays up so Sunshine can initialize streams.
KDE Plasma, audio, input bridge, and user applications are started by Sunshine
when a Moonlight app starts, sized from the client's requested width, height, and
FPS where the active display backend supports it, then stopped again when the app
ends. Plasma scaling defaults to `auto`, derived from the requested stream
resolution, and can be overridden with `SUNSHINE_SCALE` such as `1`, `1.5`, or
`2`. The fallback capture display defaults to 1920x1080 before a client request
is available, and can be overridden with `SUNSHINE_WIDTH` and `SUNSHINE_HEIGHT`.
Plasma's lock and logout actions are hidden; use the Disconnect launcher on the
desktop or panel to close a Moonlight session.
