# Moonrise: one command per component. A thin wrapper over scripts/ (each script's --help says
# more); works with any POSIX make (GNU make, BSD make). `make` alone lists the targets.
#
# Variables, on the command line or in the environment:
#   PARTS=        parts for `make test` (scripts server android windows apple smb; default all but smb)
#   SERVER_IMAGE= image `make server` builds (default moonrise-server:dev, as `make server-run` uses)
#   DIR=          target folder for `make export`
#   DEV=1         the Dev app for apple-sim and apple-tv-sim (scripts/common.sh)
#   MAC=          the build Mac (user@host) for the Apple targets; on a Mac itself, local

SERVER_IMAGE ?= moonrise-server:dev
HOOKS_NOTE = [ "$$(git config --get core.hooksPath)" = .githooks ] || echo 'note: the tracked git hooks are off in this clone; run make hooks' >&2

.PHONY: help hooks check check-quick test server server-run server-stop server-logs apple-sim apple-tv-sim android export site

## help           this list
help:
	@echo 'Moonrise: make <target>'
	@echo
	@sed -n 's/^## /  /p' Makefile
	@echo
	@echo 'Building from source: docs/building.md'

## hooks          turn on the tracked git hooks (.githooks/pre-push: no push to main, no private details)
hooks:
	git config core.hooksPath .githooks

## check          everything a pull request must pass: tests, format, server image, Apple builds, export scan (scripts/check.sh)
check:
	@$(HOOKS_NOTE)
	scripts/check.sh

## check-quick    the same without the server image and the Apple builds (scripts/check.sh --quick)
check-quick:
	@$(HOOKS_NOTE)
	scripts/check.sh --quick

## test           the automatic tests only (scripts/test-all.sh; PARTS="server android" for some)
test:
	scripts/test-all.sh $(PARTS)

## server         build the server image from server/ (SERVER_IMAGE, default moonrise-server:dev)
server:
	docker build -t $(SERVER_IMAGE) server

## server-run     build and run the server from this checkout, print its address and setup code (scripts/server-dev.sh)
server-run:
	scripts/server-dev.sh up

## server-stop    stop the server from server-run; its data (.dev-data/) stays
server-stop:
	scripts/server-dev.sh down

## server-logs    follow the log of the server from server-run
server-logs:
	scripts/server-dev.sh logs

## apple-sim      build the iPhone app and launch it on a simulator (scripts/build.sh sim, scripts/sim.sh)
apple-sim:
	scripts/build.sh sim
	scripts/sim.sh

## apple-tv-sim   build the Apple TV app and launch it on a simulator (scripts/build.sh tvos, scripts/tvsim.sh)
apple-tv-sim:
	scripts/build.sh tvos
	scripts/tvsim.sh

## android        build the Android TV app (debug, Moonrise Dev) in Docker (clients/android/build.sh)
android:
	clients/android/build.sh

## export         the public copy of the committed tree, scanned for secrets (DIR=<empty folder>; scripts/export-public.sh)
export:
	@if [ -z "$(DIR)" ]; then echo 'usage: make export DIR=<empty or new folder>' >&2; exit 2; fi
	scripts/export-public.sh "$(DIR)"

## site           preview the landing page (site/) on http://127.0.0.1:8000/ (Python 3; see site/README.md)
site:
	python3 -m http.server --bind 127.0.0.1 --directory site 8000
