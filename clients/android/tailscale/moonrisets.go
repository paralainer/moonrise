// Copyright (c) the Moonrise contributors. SPDX-License-Identifier: MPL-2.0

// Package moonrisets is the Android apps' embedded Tailscale node (tsnet, userspace, no VPN): the
// app joins the tailnet as its own device and reaches the home server through local port
// forwarders. Bound to Kotlin with gomobile (scripts/build-tailscale-android.sh); the app's
// side is clients/android/shared/src/tailscale. As the Apple apps' TailscaleService.
package moonrisets

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"time"

	"tailscale.com/client/local"
	_ "tailscale.com/feature/androidbin" // fallbacks when the app's interface list is missing
	"tailscale.com/ipn"
	"tailscale.com/logtail"
	"tailscale.com/net/netmon"
	"tailscale.com/tsnet"
)

// Host is the app's side: where state changes go, and what the node can't find out by itself.
type Host interface {
	// OnState: "starting", "needslogin" (detail: the sign-in URL, or empty while it's on its
	// way), "running" (detail: this device's tailnet address) or "failed" (detail: why).
	OnState(state, detail string)
	// Interfaces lists the device's network interfaces (Android denies apps the kernel's list),
	// one per line, as parseInterfaces reads them.
	Interfaces() string
	// Log a line (the app shows them in debug builds only).
	Log(line string)
}

var (
	mu         sync.Mutex
	srv        *tsnet.Server
	lc         *local.Client
	cancel     context.CancelFunc
	forwarders = map[string]*forwarder{}
	// The app's side, for the interface getter: not under mu, which Start holds while the node
	// starts and asks for the interfaces (it deadlocked).
	host atomic.Pointer[Host]
)

func init() {
	// Nothing goes to Tailscale's log service: the node talks only to the coordination
	// server and relays, as the privacy notes say (docs/privacy.md).
	logtail.Disable()
	netmon.RegisterInterfaceGetter(func() ([]netmon.Interface, error) {
		h := host.Load()
		if h == nil {
			return nil, errors.New("no interface list yet")
		}
		return parseInterfaces((*h).Interfaces())
	})
}

// Start brings the node up with its state in dir. authKey signs it in without the web sign-in
// (an ephemeral node; debug builds only). It returns at once; progress goes to h.OnState.
func Start(dir, hostname, authKey string, verbose bool, h Host) error {
	mu.Lock()
	defer mu.Unlock()
	if srv != nil {
		return nil
	}
	host.Store(&h)
	if err := useLogsDir(dir); err != nil {
		return err
	}
	logf := func(format string, args ...any) {
		if verbose {
			h.Log(fmt.Sprintf(format, args...))
		}
	}
	s := &tsnet.Server{
		Dir:       dir,
		Hostname:  hostname,
		AuthKey:   authKey,
		Ephemeral: authKey != "",
		Logf:      logf,
		UserLogf:  logf,
	}
	h.OnState("starting", "")
	if err := s.Start(); err != nil {
		return err
	}
	c, err := s.LocalClient()
	if err != nil {
		s.Close()
		return err
	}
	ctx, cc := context.WithCancel(context.Background())
	srv, lc, cancel = s, c, cc
	go watch(ctx, s, c, h)
	return nil
}

// useLogsDir keeps the node's log state (its log id, the sockstat buffer) in dir/logs. Without it
// the node looks for a system or cache directory, and in an app's sandbox finds none and panics.
// Nothing in it is sent anywhere: log uploads are off (init).
func useLogsDir(dir string) error {
	logs := filepath.Join(dir, "logs")
	if err := os.MkdirAll(logs, 0o700); err != nil {
		return err
	}
	return os.Setenv("TS_LOGS_DIR", logs)
}

// watch follows the node's state, as the Apple apps' TailscaleService.handle.
func watch(ctx context.Context, s *tsnet.Server, c *local.Client, h Host) {
	var w *local.IPNBusWatcher
	var err error
	for ctx.Err() == nil {
		if w, err = c.WatchIPNBus(ctx, ipn.NotifyInitialState|ipn.NotifyNoPrivateKeys); err == nil {
			break
		}
		time.Sleep(500 * time.Millisecond)
	}
	if w == nil {
		return
	}
	defer w.Close()
	loginAsked := false
	for {
		n, err := w.Next()
		if err != nil {
			if ctx.Err() == nil {
				h.OnState("failed", err.Error())
			}
			return
		}
		if n.BrowseToURL != nil && *n.BrowseToURL != "" {
			h.OnState("needslogin", *n.BrowseToURL)
		}
		if n.ErrMessage != nil && *n.ErrMessage != "" {
			h.Log("tailscale: " + *n.ErrMessage)
		}
		if n.State == nil {
			continue
		}
		switch *n.State {
		case ipn.NeedsLogin:
			if n.BrowseToURL == nil {
				h.OnState("needslogin", "")
			}
			if !loginAsked {
				loginAsked = true
				go c.StartLoginInteractive(ctx)
			}
		case ipn.NeedsMachineAuth:
			h.OnState("failed", "Approve this device in the Tailscale admin console.")
		case ipn.Running:
			loginAsked = false
			ip4, ip6 := s.TailscaleIPs()
			ip := ip4.String()
			if !ip4.IsValid() {
				ip = ip6.String()
			}
			h.OnState("running", ip)
		}
	}
}

// Stop closes the node and its forwarders. Its sign-in stays in dir.
func Stop() {
	mu.Lock()
	s, cc := srv, cancel
	fs := forwarders
	srv, lc, cancel = nil, nil, nil
	forwarders = map[string]*forwarder{}
	mu.Unlock()
	for _, f := range fs {
		f.Close()
	}
	if cc != nil {
		cc()
	}
	if s != nil {
		s.Close()
	}
}

type peer struct {
	HostName string   `json:"host_name"`
	DNSName  string   `json:"dns_name"`
	IPs      []string `json:"ips"`
	Online   bool     `json:"online"`
	Tagged   bool     `json:"tagged"`
}

// Peers answers the tailnet's other devices as JSON: [{host_name, dns_name, ips, online, tagged}].
// It waits at most 3 s (routing can't wait for the node's own 60 s).
func Peers() (string, error) {
	mu.Lock()
	c := lc
	mu.Unlock()
	if c == nil {
		return "[]", nil
	}
	ctx, cc := context.WithTimeout(context.Background(), 3*time.Second)
	defer cc()
	st, err := c.Status(ctx)
	if err != nil {
		return "", err
	}
	out := []peer{}
	for _, p := range st.Peer {
		ips := make([]string, 0, len(p.TailscaleIPs))
		for _, a := range p.TailscaleIPs {
			ips = append(ips, a.String())
		}
		out = append(out, peer{HostName: p.HostName, DNSName: p.DNSName, IPs: ips, Online: p.Online, Tagged: p.Tags != nil && p.Tags.Len() > 0})
	}
	b, err := json.Marshal(out)
	return string(b), err
}

// Forward answers the local port that tunnels to target ("host:port") on the tailnet, opening a
// forwarder on preferredPort (0: any) the first time.
func Forward(target string, preferredPort int) (int, error) {
	mu.Lock()
	defer mu.Unlock()
	if f, ok := forwarders[target]; ok {
		return f.Port(), nil
	}
	if srv == nil {
		return 0, errors.New("tailscale isn't running")
	}
	f, err := newForwarder(target, preferredPort, defaultLimits, srv.Dial)
	if err != nil {
		return 0, err
	}
	forwarders[target] = f
	return f.Port(), nil
}

// Rebind tells the node the device's network changed: rebind its sockets and find its
// endpoints again (`tailscale debug rebind`, `restun`). Best effort, as the Apple apps' TailnetNudge.
func Rebind() {
	mu.Lock()
	c := lc
	mu.Unlock()
	if c == nil {
		return
	}
	ctx, cc := context.WithTimeout(context.Background(), 2*time.Second)
	defer cc()
	for _, a := range []string{"rebind", "restun"} {
		if err := c.DebugAction(ctx, a); err != nil {
			if h := host.Load(); h != nil {
				(*h).Log("tailscale: " + a + ": " + err.Error())
			}
		}
	}
}
