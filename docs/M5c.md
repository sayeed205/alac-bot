# M5c — Engine retarget to the live `/alac` contract

Status: planned (after M5b bot wiring). Source: @oracle architecture review of
2026-09-07/08, against the md5-pinned snapshot (`docs/oracle-snapshot/`).

## Background

M5a ported `orchestrator/rip-orchestrator.ts` faithfully — but that file is an
**unwired refactor** in the live TS codebase. The actual `/alac` command runs
an older inline pipeline (`commands-rip.ts` `executeRipPipeline`, L448+), and
only `commands/status.ts` imports the orchestrator. M5b therefore ports the
live command flow into the bot crate (handler owns conversation policy; engine
seams own I/O). M5c then consolidates: one engine orchestrator implementing
the live observable contract, and the bot handler collapses onto
`engine::orchestrator`.

Decision (user-approved 2026-09-08): ship M5b against the live-command flow
first; retarget the engine as M5c.

## Engine amendments (the observable deltas to reconcile)

| # | Area | Live command (target) | Current engine | Fix |
|---|---|---|---|---|
| 1 | Maintenance gate | Cache lookup first; uncached skipped when live-off; single-track long message vs multi-track summary (rip.ts:1013-1047) | Early `can_rip_live` throw before cache lookup | Remove early gate; cache-first; return skipped uncached; bot renders UX |
| 2 | Resolution failures | Continue per item; accumulate `failedItems`; all-failed → structured error w/ details (rip.ts:573-687) | First-error fails job | Continue; structured `ResolutionFailed` |
| 3 | Headers | HTML-rich collection labels (rip.ts:579-715) | Plain strings | Live-command-compatible or expose display metadata for bot |
| 4 | Queue UX | `EnqueueOptions{signal, on_position_change → '⏳ In Queue: Position #N', on_start}` (rip.ts:1495-1508) | `enqueue(task, None)` | Pass options; engine updates `queue_position` + emits progress |
| 5 | Queued cancellation | Signal aborts pending item | No signal → cancelled job lingers queued | Pass `signal = job controller` (correctness, not just UX) |
| 6 | Progress text | `📥 Downloading` / `🏷️ Tagging` / `📤 Uploading` + MB-in-brackets (rip.ts:1093-1130) | `⬇️`/`⬆️` byte-bar strings | Align fragments or emit semantic data for bot formatting |
| 7 | Circuit breaker | Six exact mirror phrases (rip.ts:1154-1176) | Broad substring match | Live predicate |
| 8 | Upload retries | Configured count + 0.8–1.2 jitter + `ENTITY_BOUNDS_INVALID` caption fallback (rip.ts:1273-1357) | Fixed 4, rand*500 | Config via deps; live jitter; document ferogram caption fallback |
| 9 | Elapsed time | Starts at cache processing; all-cached reports real elapsed (rip.ts:732,817) | Starts at queueing; "0.0" | Align |
| 10 | Retry exhaustion | Throw → outer catch (status left, no summary row) | Records failure row, continues | Keep documented deviation or align; decide at M5c |
| 11 | Terminal events | Post-cancel suppression (no summary after cancel) | Can emit terminal after `Cancelled` | Exactly-one-terminal-event invariant |
| 12 | Job render context | — | Events lack `is_cache_only`/`is_group`/reply-to | Add to job snapshot/event payloads |
| 13 | Edit throttling | Bot-side (10s/dedupe/force) (rip.ts:786-809) | Engine emits only | Stays bot-side; engine may add immediate/normal hint |

## Ferogram/MTProto deviations (carried from M5b, permanent)

- Dump-channel delete: per-message `get_messages` + `delete_with` (ferogram
  `Client::delete_messages` does not support channel posts). Batch API later.
- File identity: no Bot-API `file_id`/`file_unique_id` in MTProto. Store
  versioned strings (`mtproto:document:<id>`, `mtproto:v1:<dc>:<id>:<hash>`).
  Old Bot-API rows cannot be matched to MTProto docs without reindexing;
  `message_id` + dump channel is the authoritative delivery reference.

## Sequencing

1. Golden-string tests first (freeze live behavior before amending engine).
2. Engine amendments 1–5, 11 (semantics) → amend M5a tests to the live
   contract (supersede refactor-only expectations).
3. Amendments 6–10, 12–13 (presentation/data plumbing).
4. Collapse the M5b bot inline pipeline onto `engine::orchestrator`
   (handler keeps parsing/gates/preflight/status editing only).
5. Full gates + commit as M5c.

## Settled status/dashboard UX (2026-09-08)

- `/status` is authorized-user-only and presents a global FIFO queue across
  DMs, groups, and channels. Requester display names are visible.
- Each chat has one live dashboard. A subsequent `/status` sends a replacement
  first and deletes the older dashboard only after the replacement succeeds.
- Every job change refreshes all registered dashboards, subject to per-message
  coalescing/flood control. Telegram flood waits pause edits for the requested
  period, log the event, and send one warning in the affected dashboard chat.
- Five jobs per page with inline previous/next/refresh controls. Rows show
  requester, job header, `▶ Processing` or waiting `#N`, percent, and
  cached/ripped/failed/total counts. There are no job-detail or failure pages.
- Header displays ripping mode, global queue length, and last-known mirror
  health (`unknown` initially). The terminal empty state is exactly
  `✅ <b>No active downloads.</b>` and stops refreshing after its first edit.
- Cancel controls render only for the requester or an admin. Cancellation
  delivers a final `🛑 Cancelled` summary including work already
  cached/ripped/failed.
- `/alac` has no separate long-lived progress message; the shared dashboard is
  canonical. Completion: group requests get a compact group notice plus a full
  requester-DM summary; DM requests get the full DM summary.

## Implementation record (M5c, 2026-09-08)

- `3782adb` — bot: live global `/status` dashboard (snapshot rendering,
  pagination, requester-aware cancel controls, flood coalescing).
- `d401684` — engine: orchestrator retargeted to live `/alac` semantics
  (cache-first maintenance, structured resolution failures, queue callbacks,
  six-phrase circuit breaker, upload retries with jitter, measured elapsed).
- `6658327` — bot: M5b inline pipeline collapsed onto the orchestrator.
  `handlers/rip` keeps policy/gates/preflight/input only; `event_bridge.rs`
  is the single rendering path for status messages and dashboard refreshes;
  `dashboard_map.rs` maps engine snapshots to viewer-scoped rows.

## Known deviations from the live oracle (intentional)

- **Retry exhaustion**: the TS oracle throws on upload-retry exhaustion and
  rejects the whole `Promise.all` batch; Rust records the failed track and
  continues later tracks, completing with a failure summary.
- **Circuit-breaker stall**: on a mirror-offline break the TS producer leaks
  its job + queue slot (blocked forever on a full bounded channel); Rust
  settles cleanly via mpsc sender-drop with the same observable summary.
- **Mirror health**: the dashboard header shows `unknown` until a health
  probe integration is wired (last-known value only; no polling yet).
- **Group completion notice**: the engine summary is edited into the
  requester-chat status message; the compact group notice + DM-summary pair
  is delivered as one message rather than the oracle's two-target split.

