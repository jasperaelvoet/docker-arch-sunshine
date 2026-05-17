REMOTE ?= root@10.10.10.122
REMOTE_DIR ?= /root/docker-games

.PHONY: dev clean remote-sync remote-rebuild remote-dev remote-watch

dev:
	docker compose -f compose.yaml down --remove-orphans
	docker compose -f compose.yaml up -d --build sunshine
	docker attach docker-arch-sunshine

clean:
	docker compose -f compose.yaml down --remove-orphans
	rm -rf ./mnt/user_data ./mnt/arch-root

remote-sync:
	REMOTE="$(REMOTE)" REMOTE_DIR="$(REMOTE_DIR)" ./scripts/remote-sync once

remote-rebuild:
	REMOTE="$(REMOTE)" REMOTE_DIR="$(REMOTE_DIR)" ./scripts/remote-sync rebuild

remote-dev:
	REMOTE="$(REMOTE)" REMOTE_DIR="$(REMOTE_DIR)" ./scripts/remote-sync dev

remote-watch:
	REMOTE="$(REMOTE)" REMOTE_DIR="$(REMOTE_DIR)" ./scripts/remote-sync watch
