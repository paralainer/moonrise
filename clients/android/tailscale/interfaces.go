// Copyright (c) the Moonrise contributors. SPDX-License-Identifier: MPL-2.0

package moonrisets

import (
	"fmt"
	"net"
	"strconv"
	"strings"

	"tailscale.com/net/netmon"
)

// parseInterfaces reads the app's interface list: one interface per line,
//
//	name|index|mtu|flags|addr/prefix,addr/prefix
//
// where flags holds u (up), l (loopback), m (multicast) and p (point-to-point).
func parseInterfaces(s string) ([]netmon.Interface, error) {
	var out []netmon.Interface
	for _, line := range strings.Split(s, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		f := strings.Split(line, "|")
		if len(f) != 5 {
			return nil, fmt.Errorf("interface line %q: want 5 fields", line)
		}
		index, err1 := strconv.Atoi(f[1])
		mtu, err2 := strconv.Atoi(f[2])
		if err1 != nil || err2 != nil {
			return nil, fmt.Errorf("interface line %q: bad index or mtu", line)
		}
		var flags net.Flags
		for _, c := range f[3] {
			switch c {
			case 'u':
				flags |= net.FlagUp | net.FlagRunning
			case 'l':
				flags |= net.FlagLoopback
			case 'm':
				flags |= net.FlagMulticast
			case 'p':
				flags |= net.FlagPointToPoint
			}
		}
		addrs := []net.Addr{}
		for _, a := range strings.Split(f[4], ",") {
			if a == "" {
				continue
			}
			_, ipnet, err := net.ParseCIDR(a)
			if err != nil {
				return nil, fmt.Errorf("interface %s: address %q: %v", f[0], a, err)
			}
			ip, _, _ := net.ParseCIDR(a)
			ipnet.IP = ip
			addrs = append(addrs, ipnet)
		}
		out = append(out, netmon.Interface{
			Interface: &net.Interface{Index: index, MTU: mtu, Name: f[0], Flags: flags},
			AltAddrs:  addrs,
		})
	}
	return out, nil
}
