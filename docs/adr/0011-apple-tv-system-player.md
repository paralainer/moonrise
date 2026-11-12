# 0011. Apple TV uses the system player

Status: accepted

## Context

tvOS users know the system player's controls, Siri Remote gestures, audio menus and scrub
preview. A custom player would have to rebuild all of that and still feel foreign.

## Decision

- On Apple TV, playback uses `AVPlayerViewController`. Moonrise's extras plug into its
  extension points: Next Episode and Episodes as tabs in the swipe-down row
  (`customInfoViewControllers`), extra menus in the transport bar
  (`transportBarCustomMenuItems`). Nothing is drawn over the video that would need focus.
- While loading: a plain black screen with the system spinner.
- The scrub preview comes from the server's I-frame playlist (ADR 0004).
- iPhone, iPad and Mac use Moonrise's own player, which also plays in the title page's header.

## Consequences

- Some layouts are constrained by AVKit (buttons over the player can't take focus while the
  transport bar is up).
- Focus and press handling in these panels can only be checked on a simulator or device.
