# syntax=docker/dockerfile:1.7

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
      htop \
      jack2 \
      kate \
      kcalc \
      kde-cli-tools \
      kdf \
      khelpcenter \
      kinfocenter \
      kio-extras \
      ksystemlog \
      konsole \
      kscreen \
      kwin \
      libxcvt \
      lib32-mesa \
      lib32-vulkan-radeon \
      libva-mesa-driver \
      libva-utils \
      mesa \
      mesa-utils \
      nano \
      noto-fonts \
      noto-fonts-emoji \
      okular \
      plasma-desktop \
      plasma-integration \
      plasma-systemmonitor \
      plasma-workspace \
      plasma-x11-session \
      procps-ng \
      plasma-nm \
      plasma-pa \
      qt6-multimedia-ffmpeg \
      seatd \
      shared-mime-info \
      spectacle \
      steam \
      sudo \
      systemsettings \
      vulkan-radeon \
      vulkan-tools \
      wl-clipboard \
      xdg-desktop-portal \
      xdg-desktop-portal-kde \
      xdg-user-dirs \
      xdg-utils \
      xf86-input-libinput \
      xf86-video-dummy \
      xorg-server-xvfb \
      xorg-server \
      xorg-xauth \
      xorg-xdpyinfo \
      xorg-xinit \
      xorg-xrandr \
      xorg-xset \
      xorg-xsetroot \
      xorg-xwayland \
      xterm \
    && curl -fsSL -o /tmp/sunshine.pkg.tar.zst "https://github.com/LizardByte/Sunshine/releases/download/v${SUNSHINE_VERSION}/sunshine-${SUNSHINE_VERSION}-1-x86_64.pkg.tar.zst" \
    && printf '\nLocalFileSigLevel = Never\n' >> /etc/pacman.conf \
    && pacman -U --noconfirm /tmp/sunshine.pkg.tar.zst \
    && rm -f /tmp/sunshine.pkg.tar.zst \
    && groupadd -f input \
    && groupadd -f render \
    && useradd -m -u 1000 -s /bin/bash -G wheel,video,input,render sunshine \
    && printf 'sunshine:sunshine\n' | chpasswd \
    && printf '%s\n' \
      'sunshine ALL=(root) NOPASSWD: /usr/local/bin/arch-sunshine session-start, /usr/local/bin/arch-sunshine session-start *, /usr/local/bin/arch-sunshine session-stop' \
      > /etc/sudoers.d/sunshine \
    && chmod 0440 /etc/sudoers.d/sunshine \
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

COPY config/sunshine/ /usr/share/arch-sunshine/sunshine/
COPY --chmod=755 build/container/bin/arch-sunshine-pacman /usr/local/bin/arch-sunshine-pacman
COPY --chmod=755 build/container/bin/arch-sunshine-steam /usr/local/bin/arch-sunshine-steam
COPY --chmod=755 build/container/bin/arch-sunshine-steamos-update /usr/local/bin/arch-sunshine-steamos-update
COPY --chmod=755 build/container/bin/arch-sunshine /usr/local/bin/arch-sunshine
COPY --chmod=755 build/container/bin/arch-sunshine-input-bridge /usr/local/bin/arch-sunshine-input-bridge

RUN install -d -m 1777 /var/lib/xkb \
    && mkdir -p /mnt/user_data \
    && chmod 1777 /tmp \
    && install -d -m 755 /usr/lib/arch-sunshine \
    && mv /usr/bin/pacman /usr/lib/arch-sunshine/pacman.real \
    && ln -sf /usr/local/bin/arch-sunshine-pacman /usr/bin/pacman \
    && ln -sf /usr/local/bin/arch-sunshine-steam /usr/local/bin/steam \
    && ln -sf /usr/local/bin/arch-sunshine-steamos-update /usr/bin/steamos-update \
    && if [ -f /usr/share/applications/steam.desktop ]; then \
      sed -i 's@Exec=/usr/bin/steam@Exec=/usr/local/bin/arch-sunshine-steam@g' /usr/share/applications/steam.desktop; \
    fi

EXPOSE 47984-47990/tcp
EXPOSE 48010/tcp
EXPOSE 47998-48000/udp
EXPOSE 48002/udp
EXPOSE 48010/udp

WORKDIR /

ENTRYPOINT ["/usr/local/bin/arch-sunshine"]
