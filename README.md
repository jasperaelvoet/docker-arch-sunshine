# Arch Sunshine

Barebones Arch Linux desktop container with Sunshine, KDE Plasma, Steam, and
Firefox.

## Usage

```sh
make dev
```

This builds the image, starts the container, and attaches to the small control
UI. Pair from Moonlight, then press `p` in the terminal UI to enter the pairing
PIN.

Persistent user data lives in:

```text
./mnt/user_data
```

## Commands

```sh
make dev    # build, start, and attach
make clean  # stop and remove persisted local data
```

## Defaults

- Desktop user: `sunshine`
- Desktop password: `sunshine`
- Sunshine Web UI: `https://<host-ip>:47990`
- Web UI login: `sunshine` / `sunshine`

The container uses the mounted GPU when available. On a headless GPU it runs
KDE Plasma on GPU-backed Xwayland so Sunshine can still capture an X11 desktop.
