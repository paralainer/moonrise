// Copyright (c) the Moonrise contributors. SPDX-License-Identifier: MPL-2.0

package moonrisets

import (
	"context"
	"io"
	"net"
	"strconv"
	"testing"
	"time"
)

// echoServer answers every connection with what it reads, upper-cased by nothing: a plain echo.
func echoServer(t *testing.T) string {
	ln, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ln.Close() })
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			go func() { io.Copy(c, c); c.Close() }()
		}
	}()
	return ln.Addr().String()
}

func plainDial(ctx context.Context, network, address string) (net.Conn, error) {
	var d net.Dialer
	return d.DialContext(ctx, network, address)
}

func connect(t *testing.T, f *forwarder) net.Conn {
	c, err := net.Dial("tcp4", "127.0.0.1:"+strconv.Itoa(f.Port()))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

func TestForwardsBothWays(t *testing.T) {
	f, err := newForwarder(echoServer(t), 0, defaultLimits, plainDial)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	c := connect(t, f)
	c.Write([]byte("hello"))
	c.(*net.TCPConn).CloseWrite()
	got, err := io.ReadAll(c)
	if err != nil || string(got) != "hello" {
		t.Fatalf("got %q, %v", got, err)
	}
}

// A dial that never comes back must not leave the player waiting on silence.
func TestDialTimeoutClosesTheConnection(t *testing.T) {
	limits := defaultLimits
	limits.DialTimeout = 200 * time.Millisecond
	hang := func(ctx context.Context, network, address string) (net.Conn, error) {
		<-ctx.Done()
		return nil, ctx.Err()
	}
	f, _ := newForwarder("100.64.0.1:8097", 0, limits, hang)
	defer f.Close()
	c := connect(t, f)
	c.SetReadDeadline(time.Now().Add(3 * time.Second))
	start := time.Now()
	_, err := c.Read(make([]byte, 1))
	if err == nil || time.Since(start) > 2*time.Second {
		t.Fatalf("read: %v after %v, want a close soon after the dial timeout", err, time.Since(start))
	}
	time.Sleep(50 * time.Millisecond)
	if _, _, timedOut, _ := f.counts(); timedOut != 1 {
		t.Fatalf("timed out = %d, want 1", timedOut)
	}
}

func TestRefusesPastTheDialLimit(t *testing.T) {
	limits := defaultLimits
	limits.MaxDialsInFlight = 1
	release := make(chan struct{})
	hang := func(ctx context.Context, network, address string) (net.Conn, error) {
		select {
		case <-release:
		case <-ctx.Done():
		}
		return nil, context.Canceled
	}
	f, _ := newForwarder("100.64.0.1:8097", 0, limits, hang)
	defer f.Close()
	defer close(release)
	connect(t, f)
	time.Sleep(50 * time.Millisecond)
	second := connect(t, f)
	second.SetReadDeadline(time.Now().Add(2 * time.Second))
	if _, err := second.Read(make([]byte, 1)); err != io.EOF {
		t.Fatalf("second connection: %v, want closed at once", err)
	}
	if _, _, _, refused := f.counts(); refused != 1 {
		t.Fatalf("refused = %d, want 1", refused)
	}
}

// One side ended and the other went silent (a dead tunnel): both are closed after DrainIdle.
func TestDrainIdleEndsAHalfDeadBridge(t *testing.T) {
	limits := defaultLimits
	limits.DrainIdle = 200 * time.Millisecond
	ln, _ := net.Listen("tcp4", "127.0.0.1:0")
	defer ln.Close()
	go func() {
		c, err := ln.Accept()
		if err == nil {
			defer c.Close()
			time.Sleep(5 * time.Second) // never answers, never closes
		}
	}()
	f, _ := newForwarder(ln.Addr().String(), 0, limits, plainDial)
	defer f.Close()
	c := connect(t, f)
	c.(*net.TCPConn).CloseWrite()
	c.SetReadDeadline(time.Now().Add(3 * time.Second))
	start := time.Now()
	if _, err := c.Read(make([]byte, 1)); err == nil || time.Since(start) > 2*time.Second {
		t.Fatalf("read: %v after %v, want closed after the drain wait", err, time.Since(start))
	}
}

func TestPreferredPortThenAnyPort(t *testing.T) {
	a, err := newForwarder(echoServer(t), 0, defaultLimits, plainDial)
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()
	// The port is taken: another one.
	b, err := newForwarder(echoServer(t), a.Port(), defaultLimits, plainDial)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()
	if b.Port() == a.Port() {
		t.Fatal("two forwarders on one port")
	}
	port := b.Port()
	b.Close()
	time.Sleep(50 * time.Millisecond)
	c, err := newForwarder(echoServer(t), port, defaultLimits, plainDial)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	if c.Port() != port {
		t.Fatalf("port %d, want the free preferred %d", c.Port(), port)
	}
}

func TestParseInterfaces(t *testing.T) {
	got, err := parseInterfaces("wlan0|30|1500|um|192.168.1.23/24,fe80::1/64\nlo|1|65536|ul|127.0.0.1/8\n")
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].Name != "wlan0" || got[0].Index != 30 || !got[0].IsUp() || got[0].IsLoopback() || !got[1].IsLoopback() {
		t.Fatalf("got %+v", got)
	}
	addrs, _ := got[0].Addrs()
	if len(addrs) != 2 || addrs[0].String() != "192.168.1.23/24" {
		t.Fatalf("addrs %v", addrs)
	}
	if _, err := parseInterfaces("wlan0|x|1500|u|"); err == nil {
		t.Fatal("a bad line was accepted")
	}
}
