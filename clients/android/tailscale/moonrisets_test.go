// Copyright (c) the Moonrise contributors. SPDX-License-Identifier: MPL-2.0

package moonrisets

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"tailscale.com/logpolicy"
	"tailscale.com/net/netmon"
)

type fakeHost struct{}

func (fakeHost) OnState(state, detail string) {}
func (fakeHost) Interfaces() string            { return "wlan0|30|1500|um|192.168.1.23/24" }
func (fakeHost) Log(line string)               {}

// Start holds mu while the node starts, and the node asks for the interfaces then: the
// getter must not need mu (it did, and the node never came up).
func TestTheInterfaceGetterDoesntWaitForStart(t *testing.T) {
	var h Host = fakeHost{}
	host.Store(&h)
	defer host.Store(nil)
	mu.Lock()
	defer mu.Unlock()
	got := make(chan netmon.InterfaceList, 1)
	go func() {
		l, _ := netmon.GetInterfaceList()
		got <- l
	}()
	select {
	case l := <-got:
		if len(l) != 1 || l[0].Name != "wlan0" {
			t.Fatalf("interfaces %+v, want the app's list", l)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("the interface getter waited for Start")
	}
}

// In an app's sandbox the node found no directory for its log state and panicked,
// taking the app with it; it now uses one in the app's own folder.
func TestTheNodesLogStateStaysInTheAppsFolder(t *testing.T) {
	dir := t.TempDir()
	defer os.Unsetenv("TS_LOGS_DIR")
	if err := useLogsDir(dir); err != nil {
		t.Fatal(err)
	}
	if got, want := logpolicy.LogsDir(t.Logf), filepath.Join(dir, "logs"); got != want {
		t.Fatalf("logs dir %q, want %q", got, want)
	}
}
