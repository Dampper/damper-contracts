# 0003. Only a keeper can move the anchor

Status: accepted, 2026-11-18

## Context

The anchor has to follow real price changes, so something has to move it. The first version let anyone call `poke`, for liveness. Each poke sets the anchor to the current clamped quote, so whoever pokes chooses how fast the anchor compounds. Under a held 5% per hour pump, poking every five minutes takes the anchor to 3.3x in a day. If nobody pokes, the band stops widening at the window and the quote stops at 2.2x. Permissionless poking handed the attacker the faster path.

## Decision

`poke` requires the keeper's authorisation. The keeper is the admin until `set_keeper` is called. A poke within 300 seconds of the last move does nothing, so a keeper cannot be made to freeze the anchor by being poked constantly either.

## Consequences

- An attacker cannot choose the anchor's speed.
- If the keeper stops, the anchor goes stale and quotes become more cautious, never less. The cost is that real rallies stop being tracked.
- A stolen keeper key can move the anchor only at the policy's rate, and can be replaced with `set_keeper`.
- The keeper has to run. `poke` also extends the expiry of everything a price lookup reads, so a missing keeper eventually means a missing price as well.
