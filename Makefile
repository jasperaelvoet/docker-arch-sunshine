.PHONY: dev clean

dev:
	docker compose -f compose.yaml down --remove-orphans
	docker compose -f compose.yaml up -d --build sunshine
	docker attach arch-sunshine

clean:
	docker compose -f compose.yaml down --remove-orphans
	rm -rf ./mnt/user_data ./mnt/arch-root
