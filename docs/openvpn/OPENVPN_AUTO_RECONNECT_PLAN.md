# OpenVPN Auto-Reconnect Plan

## Goal

Automatically restart OpenVPN after any **non-user** exit, with configurable exponential backoff.

## Retry policy

For consecutive failures:

- retry 1: `0s`
- retry 2: `5s`
- retry 3: `15s`
- retry 4: `35s`
- retry 5: `75s`

Generalized:

- if `failureCount == 1` → `0`
- else:
  - `delay = baseSeconds * (2^(failureCount - 1) - 1)`
  - clamp to `maxSeconds`

Defaults:
- `enabled = false`
- `baseSeconds = 5`
- `maxSeconds = 3600`

## Behavior rules

### Reconnect when
- OpenVPN exits for any reason **except** explicit user stop

### Do not reconnect when
- user clicked Stop
- app shutdown is happening
- auto-reconnect is disabled
- config is invalid and restart cannot be attempted

## New config fields

Add under OpenVPN config:

- `autoReconnectEnabled: bool`
- `autoReconnectBaseSeconds: u64`
- `autoReconnectMaxSeconds: u64`

## UI changes

### Config page
Add OpenVPN reconnect controls:
- checkbox: `Reconnect automatically after unexpected exit`
- number input: `Reconnect base delay (seconds)`
- number input: `Reconnect max delay (seconds)`

### Status/OpenVPN page
Show:
- reconnect enabled/disabled
- consecutive failure count
- next retry time/countdown
- current reconnect state

## Backend state

Add a dedicated store, e.g.

- `src-tauri/src/stores/openvpn_reconnect_store.rs`

Track:
- `consecutive_failures`
- `next_retry_at_ms`
- `retry_generation`
- `user_requested_stop`
- `pending_retry` flag/cancel token

## Service helpers

In `services/openvpn_service.rs` add:

- `compute_openvpn_reconnect_delay_secs(failure_count, base_secs, max_secs)`
- `schedule_openvpn_reconnect(...)`
- `cancel_openvpn_reconnect(...)`
- `mark_openvpn_user_stop(...)`
- `clear_openvpn_user_stop(...)`
- `handle_openvpn_unexpected_exit(...)`

## Integration points

### On manual start
- clear pending retry
- clear user-stop flag
- maybe reset failure counter on successful start

### On manual stop
- set user-stop flag
- cancel pending retry
- prevent reconnect

### On PTY/helper exit
If OpenVPN session ends:
- if user-stop flag is set:
  - do nothing
- else if auto-reconnect enabled:
  - increment failure count
  - compute delay
  - schedule restart
  - update function status to reconnect/backoff

### On successful restart launch
- clear pending retry
- keep or reset failure count depending on desired semantics

## Recommended reset semantics

Simplest v1:
- reset failure count to `0` as soon as a new OpenVPN process starts successfully

Alternative:
- reset only after a minimum stable runtime

Recommendation:
- **reset on successful start** for v1

## Status model additions

You may want to extend OpenVPN-related status data with:
- `autoReconnectEnabled`
- `reconnectFailureCount`
- `nextRetryAtMs`
- `lastExitReason`
- `reconnectState`

If not in `FunctionStatus`, keep it in a separate OpenVPN runtime query first.

## Suggested function status messages

- `OpenVPN reconnecting now`
- `OpenVPN retry scheduled in 15s`
- `OpenVPN retry scheduled in 75s`
- `OpenVPN retry backoff capped at 3600s`

## Implementation order

### Phase R1
- add config fields + defaults + persistence

### Phase R2
- add reconnect store

### Phase R3
- implement delay calculation helper

### Phase R4
- wire manual start/stop to clear/set reconnect flags

### Phase R5
- hook non-user OpenVPN exit into scheduler

### Phase R6
- update UI/status display

### Phase R7
- later expose reconnect metadata in MCP/helper

## Edge cases

- app closes while retry timer pending
- multiple rapid exits creating duplicate timers
- stale timer fires after manual start
- config changed while retry pending
- Windows helper flow and non-Windows PTY flow both need same reconnect policy

## Safety mechanism

Use a `retry_generation` counter:
- every stop/start increments generation
- scheduled retry only runs if generation still matches
- avoids stale retry timers firing later
