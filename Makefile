BUILD_TYPE ?= release

DAEMON_DIR := daemon
BUILD_DIR := build

DAEMON_BIN_DIR := /usr/bin
FCITX5_LIB_DIR := /usr/lib/fcitx5
FCITX5_ADDON_DIR := /usr/share/fcitx5/addon

ifeq ($(BUILD_TYPE),release)
  CARGO_FLAG := --release
  CARGO_TARGET := release
  CMAKE_TYPE := Release
else
  CARGO_FLAG :=
  CARGO_TARGET := debug
  CMAKE_TYPE := Debug
endif

.PHONY: daemon plugin install-daemon install-plugin dev restart clean uninstall-dev help

help:
	@echo "Usage: make <target> [BUILD_TYPE=release|debug]"
	@echo ""
	@echo "Targets:"
	@echo "  dev            Build and install everything"
	@echo "  restart        Build, install, restart fcitx5 + glm-asrd"
	@echo "  daemon         Build Rust daemon only"
	@echo "  plugin         Build C++ fcitx5 plugin only"
	@echo "  clean          Remove build artifacts"
	@echo "  uninstall-dev  Remove user-level dev plugin (revert to system package)"
	@echo ""
	@echo "Options:"
	@echo "  BUILD_TYPE=release|debug  (default: release)"

daemon:
	cd $(DAEMON_DIR) && cargo build $(CARGO_FLAG) --features vello-renderer

plugin:
	cmake -S $(CURDIR) -B $(BUILD_DIR) \
		-DCMAKE_BUILD_TYPE=$(CMAKE_TYPE) \
		-DCMAKE_INSTALL_PREFIX=/usr
	make -C $(BUILD_DIR)

install-daemon: daemon
	sudo install -Dm755 $(DAEMON_DIR)/target/$(CARGO_TARGET)/glm-asrd $(DAEMON_BIN_DIR)/glm-asrd
	sudo install -Dm755 $(DAEMON_DIR)/target/$(CARGO_TARGET)/glm-asr-overlay $(DAEMON_BIN_DIR)/glm-asr-overlay

install-plugin: plugin
	sudo install -Dm755 $(BUILD_DIR)/plugin/glm-asr.so $(FCITX5_LIB_DIR)/glm-asr.so
	sudo install -Dm644 $(BUILD_DIR)/plugin/glm-asr.conf $(FCITX5_ADDON_DIR)/glm-asr.conf

dev: install-daemon install-plugin

restart: dev
	systemctl --user restart glm-asrd.service 2>/dev/null || true
	pkill -x fcitx5 2>/dev/null || true
	@sleep 1
	@nohup fcitx5 -d > /dev/null 2>&1 &
	@echo "Done. fcitx5 and glm-asrd restarted."

uninstall-dev:
	sudo rm -f $(FCITX5_LIB_DIR)/glm-asr.so
	sudo rm -f $(FCITX5_ADDON_DIR)/glm-asr.conf
	@echo "Dev plugin removed. Reinstall system package to restore: pacman -S fcitx5-glm-asr"

clean:
	rm -rf $(BUILD_DIR)
	cd $(DAEMON_DIR) && cargo clean
