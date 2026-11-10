<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0023: Four alert levels for the (i) icon

- **Status:** accepted 2026-11-08.
  - Owner decision, 0.2.5 feedback, 2026-11-08 (T54).
  - The Code Orchestrator OKed the contract change.
  - The Security and Network Expert approved it with R6 and R7 (review notes of 2026-11-08, not in the repository).
- **Date:** 2026-11-08
- **Spec reference:** rules.md (numbered error codes, a severity per code); block 10 (status legend, updated 2026-11-08 by the UX Designer to point to this ADR).

## Context

The spec (block 10) maps health into three states: Healthy, Needs attention, Action required. The error catalog carried two severities to match: `needs-attention` and `action-required`.

On 2026-11-08 the owner asked for four, each with its own colour of the (i) icon:
- **white:** user, please note;
- **yellow:** needs handling;
- **red:** needs an immediate answer;
- **flashing red with a yellow frame:** the most serious fault. Do not go on using the archive until it is fixed (the backup is at risk).

The home screen turns from blue to red when something is wrong. This replaced the spec's three states. The UX Designer updated block 10 on 2026-11-08 (UX addendum 0.2.5): the legend is All fine, Note, Action needed, Respond now, Critical. The repo copy (`docs/spec.md`) carries the same text.

## Decision

**Levels.** `note`, `action`, `now`, `critical`, least to most serious. In the spec legend they are Note, Action needed, Respond now and Critical. "All fine" is the absence of alerts (`state.status.level = "ok"`).

**Disk health** (spec block 10 disk mapping):
- Healthy: All fine (no level).
- Showing wear: Action needed (`action`).
- Failing: Respond now (`now`), or Critical (`critical`) when the failing disk holds the archive.
- `live/disks` gives each disk this `level` next to its `health`. The archive disk's standing alert follows the same mapping: `archive.disk_wear` is `action`, `archive.disk_failing` is `critical`.

**The catalog.** Every entry's `severity` is one of the four. The CI check (`crates/alp-core/tests/error_catalog.rs`) allows only these values. Each code was assigned by what it means, not by a blanket rename:

| Level | Codes |
|---|---|
| `note` | an update rolled back (BOOT-001), updates not downloaded (UPD-001), the clock not set (NET-002), pairing code wrong (PAIR-001), app kept at its version (APP-002), power loss (PWR-001), the hardware report's optional items (HW-103, HW-104, HW-107, HW-108, HW-109), an adapter without a driver (NET-102) |
| `action` | a disk wearing out (STO-002), the system disk wearing out (HW-001), refusals of a chosen disk (STO-100 to STO-109, 115, 120, 123, 124, 125), storage almost full (STO-003), a storage change stopped (STO-007), the installer's refusals, key to save (KEY-001), cable or adapter missing (NET-100, NET-101), hardware too small or too old (HW-100 to HW-106) |
| `now` | a disk missing (STO-001), the archive not opened (STO-005), archive set-up, lock, test and repair failures (STO-110, 113, 114, 116, 118, 119, 121, 122), keys (KEY-002, KEY-004), no network (NET-001), the photo app not running (APP-001), an update rejected (UPD-002) |
| `critical` | damaged data that could not be repaired (STO-004), writing to the backup disk failed (STO-111), the backup disk stopped answering (STO-112), the disk's own health check reports a problem (STO-117), the activity record does not check out (KEY-003) |

**The API.**
- Every error answer has `level`: from its catalog code, otherwise from a small table by key (answers to what the person just typed are `note`), otherwise `action`.
- `state.alerts` lists the standing problems: space on the stick and the backup disk, the archive's error, Immich paused, and the archive disk's own SMART health.
- `state.status` is the most serious alert, or `{"level": "ok"}`.
- The feature is `status_levels`.

**Critical.**
- It is set only by alpd's own checks: SMART on the archive disk every 5 minutes, and the archive's own error. It is cleared only when a later check passes (R7).
- While the status is critical, alpd itself refuses deleting photos and emptying the trash (`409 archive.critical`). nginx answers new phone uploads with 503, so the app keeps the photo and tries again later.
- **Formatting another USB disk, recovery, export, Files and the disk check stay allowed:** they are the way to rescue the photos to a new disk (R6).

## Consequences

- The UI colours the (i) icon from `level` alone. No UI table of codes is needed.
- **New codes need a considered level.** CI enforces that one of the four is given.
