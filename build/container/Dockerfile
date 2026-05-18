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
      pkgconf

COPY build/container/src/ /usr/local/src/arch-sunshine/
RUN mkdir -p /out \
    && cc -O2 -pipe -Wall -Wextra -o /out/arch-sunshine-input-bridge /usr/local/src/arch-sunshine/arch-sunshine-input-bridge.c $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0 libei-1.0) \
    && cc -O2 -pipe -Wall -Wextra -o /out/arch-sunshine-server /usr/local/src/arch-sunshine/arch-sunshine-server.c \
    && strip /out/arch-sunshine-input-bridge /out/arch-sunshine-server

FROM --platform=linux/amd64 archlinux:base

USER root

ARG SUNSHINE_VERSION=2026.516.143833

RUN printf '\nDisableSandbox\n\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n' >> /etc/pacman.conf \
    && rm -rf /var/cache/pacman/pkg/* \
    && pacman-key --init \
    && pacman-key --populate archlinux \
    && pacman -Syy --noconfirm archlinux-keyring \
    && pacman -Syu --noconfirm --needed \
      ark \
      avahi \
      breeze \
      breeze-icons \
      curl \
      dbus \
      desktop-file-utils \
      dolphin \
      filelight \
      firefox \
      foot \
      grim \
      gwenview \
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
      htop \
      intel-media-driver \
      intel-media-sdk \
      jack2 \
      kate \
      kcalc \
      kactivitymanagerd \
      kde-cli-tools \
      kdf \
      khelpcenter \
      kinfocenter \
      kio-extras \
      ksystemlog \
      konsole \
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
      libva-mesa-driver \
      libva-utils \
      libei \
      libx11 \
      libxcb \
      libxfixes \
      libxrandr \
      libxtst \
      mesa \
      mesa-utils \
      miniupnpc \
      nano \
      noto-fonts \
      noto-fonts-emoji \
      numactl \
      okular \
      openssl \
      opus \
      plasma-desktop \
      plasma-integration \
      plasma-systemmonitor \
      plasma-workspace \
      procps-ng \
      plasma-nm \
      plasma-pa \
      python-cryptography \
      python-dbus \
      python-gobject \
      qt6-multimedia-ffmpeg \
      seatd \
      shared-mime-info \
      spectacle \
      steam \
      systemsettings \
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

RUN pacman -S --noconfirm --needed weston \
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
COPY --chmod=755 build/container/bin/arch-sunshine /usr/local/bin/arch-sunshine
COPY --from=native-builder /out/arch-sunshine-input-bridge /usr/local/bin/arch-sunshine-input-bridge
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
    && mv /usr/bin/pacman /usr/lib/arch-sunshine/pacman.real \
    && (setcap cap_net_raw+ep /usr/bin/ping || chmod 4755 /usr/bin/ping) \
    && setcap cap_sys_admin,cap_sys_nice+ep /usr/bin/sunshine \
    && ln -sf /usr/bin/sunshine /usr/local/bin/sunshine \
    && sed -i -E 's/^hosts:.*/hosts: files mymachines myhostname dns resolve/' /etc/nsswitch.conf \
    && printf 'precedence ::ffff:0:0/96  100\n' >> /etc/gai.conf \
    && ln -sf /usr/local/bin/arch-sunshine-pacman /usr/bin/pacman \
    && ln -sf /usr/local/bin/arch-sunshine-steam /usr/local/bin/steam \
    && ln -sf /usr/local/bin/arch-sunshine-steamos-update /usr/bin/steamos-update \
    && if [ -f /usr/share/applications/steam.desktop ]; then \
      sed -i 's@Exec=/usr/bin/steam@Exec=/usr/local/bin/arch-sunshine-steam@g' /usr/share/applications/steam.desktop; \
    fi

WORKDIR /

ENTRYPOINT ["/usr/local/bin/arch-sunshine"]
CMD ["desktop-session"]
