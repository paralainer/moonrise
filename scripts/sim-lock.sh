#!/usr/bin/env bash
# The build Mac's simulator lock, the Mac's side: the runs that boot simulators or build there for
# tests (test-apple.sh, check.sh's Apple builds, store-shots.sh) take it, so only one of them uses
# the Mac at a time; several at once made the Mac swap until timing tests failed. Scripts don't run
# this directly: sim_lock in scripts/common.sh sends it over ssh (before anything is copied there)
# and keeps that ssh open while it holds the lock.
#
#   sim-lock.sh <wait seconds> <who>
#
# It takes the lock, prints "locked", and holds it until its stdin closes: when the run is done, is
# stopped (Ctrl-C) or its ssh connection drops.
#
# The lock is the kernel's: flock(2) on the file ~/.moonrise-sim-lock, through perl (macOS's own
# has it; bash has no flock). The kernel gives it to one process at a time and takes it back when
# that process ends, however it ends (even kill -9), so there is no stale lock to detect or take
# over, and no moment in which two runs both hold it. The file itself stays; while the lock is held
# it says who holds it (PID, when, <who>), only for the messages of the runs that wait.
#
# A run that finds it held waits, saying who holds it (on stderr, then every minute), and gives up
# after <wait seconds> (exit 1). If its stdin closes while it waits (the run went away), it leaves.
# Bash 3.2 (macOS's own) is enough. No single quotes in the perl below: it is in single quotes.
set -u
WAIT=${1:?usage: sim-lock.sh <wait seconds> <who>}
WHO=${2:-a run}
exec perl -e '
use strict;
use warnings;
use Fcntl qw(:flock);
use IO::Handle;
my ($wait, $who) = @ARGV;
my $lock = "$ENV{HOME}/.moonrise-sim-lock";
open(my $fh, "+>>", $lock) or die "error: cannot open $lock: $!\n";
sub holder {
  open(my $r, "<", $lock) or return "another run";
  my @l = <$r>;
  chomp @l;
  return (defined $l[2] && $l[2] ne "") ? $l[2] : "another run";
}
my $start = time;
my $said;
until (flock($fh, LOCK_EX | LOCK_NB)) {
  my $waited = time - $start;
  if ($waited >= $wait) {
    print STDERR "error: the build Mac\x27s simulators were still in use (" . holder() . ") after $wait s of waiting: giving up\n";
    exit 1;
  }
  if (!defined $said || $waited - $said >= 60) {
    print STDERR "waiting for the build Mac\x27s simulators: in use by " . holder() . " (waited $waited s, gives up after $wait s)\n";
    $said = $waited;
  }
  my $rin = "";
  vec($rin, fileno(STDIN), 1) = 1;
  if (select(my $rout = $rin, undef, undef, 0.5) > 0) {
    my $n = sysread(STDIN, my $buf, 4096);
    exit 0 if !$n;
  }
}
truncate($fh, 0);
print $fh "$$\n" . time . "\n$who\n";
$fh->flush;
print "locked\n";
STDOUT->flush;
1 while sysread(STDIN, my $buf, 4096);
truncate($fh, 0);
close($fh);
exit 0;
' "$WAIT" "$WHO"
