// Copyright (c) the Moonrise contributors. SPDX-License-Identifier: MPL-2.0

//go:build tools

// gomobile bind needs these in the module (scripts/build-tailscale-android.sh).
package moonrisets

import (
	_ "golang.org/x/mobile/bind"
	_ "golang.org/x/mobile/cmd/gobind"
	_ "golang.org/x/mobile/cmd/gomobile"
)
