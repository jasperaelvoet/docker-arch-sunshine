# syntax=docker/dockerfile:1.7

FROM --platform=linux/amd64 archlinux:base AS native-builder

USER root

RUN printf '\nDisableSandbox\n' >> /etc/pacman.conf \
    && rm -rf /var/cache/pacman/pkg/* \
    && pacman-key --init \
    && pacman-key --populate archlinux \
    && pacman -Syy --noconfirm archlinux-keyring \
    && pacman -Syu --noconfirm --needed \
      gcc \
      glib2 \
      libei \
      libevdev \
      libcap \
      openssl \
      pkgconf

COPY build/container/src/arch-sunshine-libei-input.c /usr/local/src/arch-sunshine/arch-sunshine-libei-input.c
COPY build/container/src/arch-sunshine-server.c /usr/local/src/arch-sunshine/arch-sunshine-server.c
RUN mkdir -p /out \
    && cc -O2 -pipe -Wall -Wextra -fPIC -shared -o /out/arch-sunshine-libei-input.so /usr/local/src/arch-sunshine/arch-sunshine-libei-input.c $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0 libei-1.0 libevdev) -ldl -pthread \
    && cc -O2 -pipe -Wall -Wextra -o /out/arch-sunshine-server /usr/local/src/arch-sunshine/arch-sunshine-server.c -lcrypto -lcap \
    && strip /out/arch-sunshine-libei-input.so /out/arch-sunshine-server

FROM --platform=linux/amd64 rust:1-slim-bookworm AS rust-builder

WORKDIR /src
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache crate deps separately so source edits don't refetch/rebuild them.
COPY build/container/src/arch-sunshine-rs/Cargo.toml build/container/src/arch-sunshine-rs/Cargo.lock /src/
RUN mkdir -p /src/src && echo 'fn main() {}' > /src/src/main.rs \
    && cargo build --release --locked \
    && rm -rf /src/src /src/target/release/arch-sunshine* /src/target/release/deps/arch_sunshine*

COPY build/container/src/arch-sunshine-rs/src /src/src
RUN cargo build --release --locked \
    && install -D target/release/arch-sunshine /out/arch-sunshine

FROM --platform=linux/amd64 archlinux:base

USER root

ARG SUNSHINE_VERSION=2026.516.143833

RUN printf '\nDisableSandbox\n\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n' >> /etc/pacman.conf \
    && rm -rf /var/cache/pacman/pkg/* \
    && pacman-key --init \
    && pacman-key --populate archlinux \
    && pacman -Syy --noconfirm archlinux-keyring \
    && pacman -Syu --noconfirm --needed \
      breeze \
      breeze-icons \
      curl \
      dbus \
      desktop-file-utils \
      dolphin \
      firefox \
      gst-libav \
      gst-plugin-pipewire \
      gst-plugin-qsv \
      gst-plugin-rsrtp \
      gst-plugin-va \
      gst-plugins-bad \
      gst-plugins-base \
      gst-plugins-good \
      gst-plugins-ugly \
      gstreamer \
      intel-media-driver \
      intel-media-sdk \
      kactivitymanagerd \
      kde-cli-tools \
      kio-extras \
      kscreen \
      kwin \
      libcap \
      libdrm \
      libevdev \
      libmfx \
      libxcvt \
      lib32-mesa \
      lib32-vulkan-intel \
      lib32-vulkan-radeon \
      libpulse \
      libva-intel-driver \
      libva-utils \
      libei \
      libx11 \
      libxcb \
      libxfixes \
      libxrandr \
      libxtst \
      mesa \
      mesa-utils \
      noto-fonts \
      noto-fonts-emoji \
      numactl \
      openssl \
      opus \
      plasma-desktop \
      plasma-integration \
      plasma-workspace \
      procps-ng \
      plasma-pa \
      python-dbus \
      python-gobject \
      seatd \
      shared-mime-info \
      steam \
      vpl-gpu-rt \
      vulkan-icd-loader \
      vulkan-intel \
      vulkan-radeon \
      vulkan-tools \
      wl-clipboard \
      xdg-desktop-portal \
      xdg-desktop-portal-kde \
      xdg-user-dirs \
      xdg-utils \
      xorg-xrdb \
      xorg-xwayland \
    && curl -fsSL -o /tmp/sunshine.pkg.tar.zst "https://github.com/LizardByte/Sunshine/releases/download/v${SUNSHINE_VERSION}/sunshine-${SUNSHINE_VERSION}-1-x86_64.pkg.tar.zst" \
    && printf '\nLocalFileSigLevel = Never\n' >> /etc/pacman.conf \
    && pacman -U --noconfirm /tmp/sunshine.pkg.tar.zst \
    && rm -f /tmp/sunshine.pkg.tar.zst \
    && groupadd -f input \
    && groupadd -f render \
    && useradd -m -u 1000 -s /bin/bash -G video,input,render sunshine \
    && printf 'sunshine:sunshine\n' | chpasswd \
    && pacman -Scc --noconfirm

RUN pacman -S --noconfirm --needed \
      alsa-plugins \
      alsa-utils \
      lib32-alsa-lib \
      lib32-alsa-plugins \
      lib32-libpulse \
      lib32-pipewire \
      libpulse \
      pipewire \
      pipewire-alsa \
      pipewire-pulse \
      wireplumber \
    && pacman -Scc --noconfirm

RUN sed -i '/^\[multilib\]$/,$ { /^LocalFileSigLevel = Never$/d; }' /etc/pacman.conf

RUN sed -i 's/^#\(en_US\.UTF-8 UTF-8\)/\1/' /etc/locale.gen \
    && locale-gen \
    && printf 'LANG=en_US.UTF-8\n' > /etc/locale.conf \
    && dbus-uuidgen > /etc/machine-id

COPY --chmod=755 build/container/bin/arch-sunshine-preseed-steam /usr/local/bin/arch-sunshine-preseed-steam
RUN /usr/local/bin/arch-sunshine-preseed-steam

COPY config/sunshine/ /usr/share/arch-sunshine/
COPY --from=rust-builder --chmod=755 /out/arch-sunshine /usr/local/bin/arch-sunshine
COPY --from=native-builder /out/arch-sunshine-libei-input.so /usr/local/lib/arch-sunshine-libei-input.so
COPY --from=native-builder /out/arch-sunshine-server /usr/local/bin/arch-sunshine-server
COPY --chmod=755 build/container/bin/arch-sunshine-network-status /usr/local/bin/arch-sunshine-network-status
COPY --chmod=755 build/container/bin/arch-sunshine-pacman /usr/local/bin/arch-sunshine-pacman
COPY --chmod=755 build/container/bin/arch-sunshine-steam /usr/local/bin/arch-sunshine-steam
COPY --chmod=755 build/container/bin/arch-sunshine-steamos-update /usr/local/bin/arch-sunshine-steamos-update
COPY build/container/dbus/arch-sunshine-network-status.conf /usr/share/dbus-1/system.d/arch-sunshine-network-status.conf
COPY build/container/firefox/policies.json /usr/lib/firefox/distribution/policies.json
COPY build/container/plasma/kickoff/ /usr/share/plasma/plasmoids/org.kde.plasma.kickoff/

RUN install -d -m 1777 /var/lib/xkb \
    && mkdir -p /mnt/user_data \
    && chmod 1777 /tmp \
    && install -d -m 755 /usr/lib/arch-sunshine \
    && rm -f /usr/lib/qt6/plugins/plasma/applets/org.kde.plasma.kickoff.so \
    && rm -f /usr/share/xdg-desktop-portal/portals/gtk.portal /usr/share/xdg-desktop-portal/portals/kwallet.portal \
    && mv /usr/bin/pacman /usr/lib/arch-sunshine/pacman.real \
    && (setcap cap_net_raw+ep /usr/bin/ping || chmod 4755 /usr/bin/ping) \
    && setcap cap_sys_admin,cap_sys_nice+ep /usr/bin/sunshine \
    && ln -sf /usr/bin/sunshine /usr/local/bin/sunshine \
    && sed -i -E 's/^hosts:.*/hosts: files mymachines myhostname dns resolve/' /etc/nsswitch.conf \
    && printf 'precedence ::ffff:0:0/96  100\n' >> /etc/gai.conf \
    && ln -sf /usr/local/bin/arch-sunshine-pacman /usr/bin/pacman \
    && ln -sf /usr/local/bin/arch-sunshine-steam /usr/local/bin/steam \
    && ln -sf /usr/local/bin/arch-sunshine-steamos-update /usr/bin/steamos-update \
    && rm -f \
      /usr/share/applications/dev.lizardbyte.app.Sunshine.desktop \
      /usr/share/applications/dev.lizardbyte.app.Sunshine.terminal.desktop \
      /usr/share/applications/kaccess.desktop \
      /usr/share/applications/kcm_*.desktop \
      /usr/share/applications/kdesystemsettings.desktop \
      /usr/share/applications/org.kde.kmenuedit.desktop \
      /usr/share/applications/org.kde.knetattach.desktop \
      /usr/share/applications/org.kde.plasma.emojier.desktop \
      /usr/share/applications/systemsettings.desktop \
    && if [ -f /usr/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop ]; then \
      sed -i '/^Hidden=/d;/^NoDisplay=/d' /usr/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop; \
      printf '\nNoDisplay=true\n' >> /usr/share/applications/dev.lizardbyte.app.Sunshine.kwin.desktop; \
    fi \
    && if [ -f /usr/share/applications/steam.desktop ]; then \
      sed -i 's@Exec=/usr/bin/steam@Exec=/usr/local/bin/arch-sunshine-steam@g' /usr/share/applications/steam.desktop; \
    fi

WORKDIR /

ENTRYPOINT ["/usr/local/bin/arch-sunshine"]
CMD ["desktop-session"]
