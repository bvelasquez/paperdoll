# paperdoll — build & install
#
#   make install          # release build → ~/.local/bin/paperdoll + share assets
#   make install PREFIX=/usr/local
#   make uninstall
#
# After install, run `paperdoll` from anywhere (finds assets under
# $PREFIX/share/paperdoll, or override with PAPERDOLL_ROOT).

PREFIX   ?= $(HOME)/.local
BINDIR   := $(PREFIX)/bin
SHAREDIR := $(PREFIX)/share/paperdoll
BIN      := paperdoll
CARGO_BIN := target/release/paperdoll-app

.PHONY: all build install uninstall clean test

all: build

build:
	cargo build --release -p paperdoll-app

test:
	cargo test --workspace

install: build
	install -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(SHAREDIR)"
	install -m 755 "$(CARGO_BIN)" "$(DESTDIR)$(BINDIR)/$(BIN)"
	rm -rf "$(DESTDIR)$(SHAREDIR)/assets"
	cp -R assets "$(DESTDIR)$(SHAREDIR)/assets"
	@echo "Installed $(DESTDIR)$(BINDIR)/$(BIN)"
	@echo "Assets at  $(DESTDIR)$(SHAREDIR)/assets"
	@echo "Run: $(BIN)"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN)"
	rm -rf "$(DESTDIR)$(SHAREDIR)"
	@echo "Removed $(BIN) and $(SHAREDIR)"

clean:
	cargo clean
