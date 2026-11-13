// Copyright (c) the Moonrise contributors. SPDX-License-Identifier: MPL-2.0

package moonrisets

import (
	"context"
	"errors"
	"fmt"
	"net"
	"sync"
	"sync/atomic"
	"time"
)

// dialFunc opens a connection to a tailnet address.
type dialFunc func(ctx context.Context, network, address string) (net.Conn, error)

// forwardLimits are the forwarder's waits and caps (the Apple apps' PortForwarder.Limits).
type forwardLimits struct {
	// DialTimeout is how long a connection waits for the tunnel: long enough for a WireGuard
	// handshake to be retried once, far shorter than the player's own waits.
	DialTimeout time.Duration
	// MaxDialsInFlight is how many dials may wait at once; past it a new connection is closed at once.
	MaxDialsInFlight int32
	// DrainIdle: once one side of a connection has ended, how long the other may stay silent
	// before both are closed (a dead tunnel never ends by itself).
	DrainIdle time.Duration
}

// defaultLimits are the Apple apps' values.
var defaultLimits = forwardLimits{DialTimeout: 8 * time.Second, MaxDialsInFlight: 32, DrainIdle: 5 * time.Second}

// forwarder listens on 127.0.0.1:<port> and pipes every connection to Target on the tailnet, so
// the app's HTTP clients and the player reach a tailnet host with plain http://127.0.0.1:port URLs.
type forwarder struct {
	Target string
	ln     net.Listener
	dial   dialFunc
	limits forwardLimits

	inFlight  atomic.Int32
	bridges   atomic.Int32
	timedOut  atomic.Int32
	refused   atomic.Int32
	closeOnce sync.Once
	ctx       context.Context
	cancel    context.CancelFunc
}

// newForwarder listens on preferredPort when it is free (0: any), else on any port.
func newForwarder(target string, preferredPort int, limits forwardLimits, dial dialFunc) (*forwarder, error) {
	var ln net.Listener
	var err error
	if preferredPort > 0 {
		ln, err = net.Listen("tcp4", fmt.Sprintf("127.0.0.1:%d", preferredPort))
	}
	if ln == nil {
		ln, err = net.Listen("tcp4", "127.0.0.1:0")
	}
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithCancel(context.Background())
	f := &forwarder{Target: target, ln: ln, dial: dial, limits: limits, ctx: ctx, cancel: cancel}
	go f.acceptLoop()
	return f, nil
}

// Port is the local port it listens on.
func (f *forwarder) Port() int { return f.ln.Addr().(*net.TCPAddr).Port }

// Close stops listening and ends every dial still waiting. Open connections finish by themselves.
func (f *forwarder) Close() {
	f.closeOnce.Do(func() {
		f.cancel()
		f.ln.Close()
	})
}

// Counts: dials waiting, connections bridged, dials given up on, connections refused.
func (f *forwarder) counts() (inFlight, bridges, timedOut, refused int32) {
	return f.inFlight.Load(), f.bridges.Load(), f.timedOut.Load(), f.refused.Load()
}

func (f *forwarder) acceptLoop() {
	for {
		c, err := f.ln.Accept()
		if err != nil {
			if f.ctx.Err() != nil || errors.Is(err, net.ErrClosed) {
				return
			}
			// A bad moment (out of descriptors, an aborted connection): the listener is still there,
			// and a listener nobody accepts on leaves every new connection hanging.
			time.Sleep(100 * time.Millisecond)
			continue
		}
		if f.inFlight.Add(1) > f.limits.MaxDialsInFlight {
			f.inFlight.Add(-1)
			f.refused.Add(1)
			c.Close()
			continue
		}
		go f.connection(c)
	}
}

func (f *forwarder) connection(client net.Conn) {
	ctx, cancel := context.WithTimeout(f.ctx, f.limits.DialTimeout)
	remote, err := f.dial(ctx, "tcp", f.Target)
	cancel()
	f.inFlight.Add(-1)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) || ctx.Err() == context.DeadlineExceeded {
			f.timedOut.Add(1)
		}
		client.Close()
		return
	}
	f.bridges.Add(1)
	defer f.bridges.Add(-1)
	bridge(client, remote, f.limits.DrainIdle)
}

// bridge copies both ways until both directions end. Once one has, the other gets drainIdle of
// silence (each byte it moves starts that again), then both connections are closed.
func bridge(a, b net.Conn, drainIdle time.Duration) {
	done := make(chan struct{}, 2)
	var last atomic.Int64
	last.Store(time.Now().UnixNano())
	pump := func(from, to net.Conn) {
		buf := make([]byte, 64*1024)
		for {
			n, err := from.Read(buf)
			if n > 0 {
				last.Store(time.Now().UnixNano())
				if _, werr := to.Write(buf[:n]); werr != nil {
					break
				}
				last.Store(time.Now().UnixNano())
			}
			if err != nil {
				break
			}
		}
		// Half-close so the other direction can finish draining.
		// (A connection without half-close is left to the drain wait.)
		if tc, ok := to.(interface{ CloseWrite() error }); ok {
			tc.CloseWrite()
		}
		done <- struct{}{}
	}
	go pump(a, b)
	go pump(b, a)
	<-done
	last.Store(time.Now().UnixNano())
	tick := time.NewTicker(minDuration(drainIdle/4, 250*time.Millisecond))
	defer tick.Stop()
wait:
	for {
		select {
		case <-done:
			break wait
		case <-tick.C:
			if time.Since(time.Unix(0, last.Load())) >= drainIdle {
				// A pump in Read or Write on a dead tunnel only returns once its connection is closed.
				a.Close()
				b.Close()
				<-done
				break wait
			}
		}
	}
	a.Close()
	b.Close()
}

func minDuration(a, b time.Duration) time.Duration {
	if a < b && a > 0 {
		return a
	}
	return b
}
