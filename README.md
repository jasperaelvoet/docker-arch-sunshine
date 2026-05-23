# Docker Arch Sunshine

Arch Linux desktop container for a full KDE Plasma Wayland desktop streamed to
Moonlight through Sunshine.

## What's Included

- KDE Plasma Wayland desktop on a virtual KWin output
- Pinned Sunshine release package installed in the image
- Container-native Sunshine wrapper at `arch-sunshine-server`
- Sunshine KWin/Wayland, KMS, portal, X11, VAAPI, and Vulkan capture/encode support
- GStreamer RTP pipeline generation and live stream smoke tests for local probing
- H.264, HEVC, and AV1 encoder probing
- NVENC, VAAPI, QSV, and software encoder definitions
- PipeWire/PulseAudio session audio with an `arch_sunshine_audio` virtual sink
- Steam and Firefox launchers
- Original Sunshine app list: Desktop
- Stream-triggered client resizing from Moonlight width, height, FPS, and scale
- Disconnect launcher for closing the active Moonlight stream
- Single persistent mount model: `./mnt/user_data` to `/mnt/user_data`

## Requirements

- Docker with the Compose plugin
- Linux host with GPU devices exposed at `/dev/dri` for hardware rendering and encoding
- `/dev/uinput` and `/dev/uhid` for Sunshine keyboard, mouse, and controller input

## Usage

```sh
make dev
```

This builds the image, starts the `arch-sunshine desktop-session` runtime, and
attaches to the container. The runtime starts KDE first, then runs Sunshine
inside that same Wayland, D-Bus, and PipeWire session.

## Commands

```sh
make dev      # build, start, and attach
make clean    # stop and remove persisted local data
```

Inside the container:

```sh
arch-sunshine probe
arch-sunshine desktop-session
arch-sunshine-server pin 1234
arch-sunshine pipeline --codec h264 --width 1920 --height 1080 --fps 60
arch-sunshine disconnect
```

When Moonlight asks for pairing, attach to the container UI and press `r`, then
enter the PIN shown by Moonlight. Sunshine persists its TLS identity and paired
client records under `/mnt/user_data`; runtime config and the app list are
regenerated as read-only container-owned files on each start.

## Defaults

- Image/container: `docker-arch-sunshine`
- Desktop user: `sunshine`
- Desktop password: `sunshine`
- Persistent data: `./mnt/user_data`
- Wayland socket: `/run/user/1000/arch-sunshine-wayland`
- Desktop size: `SUNSHINE_WIDTH` x `SUNSHINE_HEIGHT`, default `1920x1080`
- Desktop scale: `SUNSHINE_SCALE`, default `auto`
- Sunshine HTTP/HTTPS ports: `47989` / `47984`
- Sunshine RTSP/media/control ports: `48010`, `47998`, `48000`, `47999`
- Sunshine capture: `SUNSHINE_CAPTURE`, default `kwin`; set `auto` to let Sunshine choose
- Sunshine encoder: `SUNSHINE_ENCODER`, default `vaapi`; set `auto` to let Sunshine choose
- Sunshine quality: lower QP fallback and higher quality GPU presets are enabled
  by default; the Moonlight client bitrate still controls normal stream bitrate
- Diagnostic RTP pipeline bitrate: `SUNSHINE_BITRATE_KBPS`, default `50000`
- KWin latency policy: `SUNSHINE_KWIN_LATENCY_POLICY`, default `Low`
- Sunshine gamepad: fixed to `xone` on Linux, the supported Xbox-style virtual pad
- KWin EIS input mirror: `ARCH_SUNSHINE_LIBEI_INPUT`, default `1`; set `0` only for raw uinput testing
- Input minimum hold: dynamic by default from the active stream FPS; set
  `ARCH_SUNSHINE_INPUT_MIN_HOLD_MS` only to force an override, including `0`
  for raw passthrough. Applies to keys, mouse buttons, controller buttons,
  D-pad, and trigger taps.

Runtime package changes are intentionally blocked. Add or remove system packages
in `build/container/Dockerfile`, then rebuild the image. Steam is seeded from
the packaged bootstrap into persistent user data on first start; client updates
and shader cache state stay under `/mnt/user_data`.

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

See `docs/wayland-gstreamer-stack.md` for the target architecture and remaining
protocol milestones.
