<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

## 2. Global engineering rules

1. **Language and tooling.** All new daemon and tool code is Rust (stable toolchain pinned via `rust-toolchain.toml`). Use `tokio`, `serde`, `rusqlite` (bundled), `tracing`, `axum` with `rustls`, `clap`. Shell is allowed only for thin glue in image build hooks and must pass `shellcheck`.
2. **No unsafe Rust** outside narrow FFI wrappers, each justified in an ADR. CI runs `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo test`, `cargo deny check` and `cargo audit`.
3. **Dependencies are a security surface.** Adding a crate requires a one-line justification in the PR description. Prefer crates that are widely used and maintained. Pin versions with `Cargo.lock` committed.
4. **Cryptography: never invent primitives or protocols.** Use `ring` or RustCrypto crates, `ed25519-dalek`, `sha3`, `zeroize`. Use an established PAKE (`spake2`) for pairing. Secrets are wrapped in zeroizing types, never logged, never written to disk in plaintext except where a block explicitly says so.
5. **Hardware isolation.** Code above the abstraction layer (Section 5) must not assume UEFI, TPM, Btrfs or any specific hardware. Only backend implementations may.
6. **Single point of access.** Only `alp-storage` may invoke `btrfs`, `cryptsetup`, `mkfs`, `smartctl`. Only `alp-crypto` handles key material. Enforce with a CI grep check.
7. **Safe defaults, no questions.** Any decision a non-technical user could not evaluate is made automatically. The UI shows a result and a recommendation.
8. **Destructive operations** (partitioning, formatting, disk removal, key rotation) require explicit confirmation from the user and are always written to the ledger.
9. **No telemetry.** The only outbound connections allowed are to the signed update server, and to nothing else.
10. **Observable and testable.** Structured logs through `tracing` to journald. Every block has a VM-based acceptance test. Failure injection is a first-class test tool.
11. **Reproducible builds.** Same inputs produce the same image hash (see block 1).

### 2.1 Upstream component policy

- Prefer integrating mature, widely used projects over writing new code. Every integrated component (Debian packages, containers such as Immich, Rust crates) is listed with its version and license in `docs/components.md`.
- **No automatic upstream updates.** Every component is pinned to an exact version (Debian snapshot mirror, container image digest, committed `Cargo.lock`). A new upstream version enters a release only after it passes the full VM test suite and a beta period on test machines. On users' machines, only signed ALP-OS images update, and only through block 2.
- Each component update is one reviewable change that summarises the upstream changelog and tests its migration steps, including rollback to the previous image.
- A component with a database or on-disk format (Immich and its PostgreSQL) must survive rollback: snapshot its data before any migration and restore it if the image rolls back. **VERIFY** Immich's migration and downgrade behaviour and record the procedure in an ADR.
- Licenses are reviewed before a component is added. Anything restricted for redistribution or commercial use is not bundled.

### 2.2 User experience rules

- **The server is the family's photo album, not a computer.** Every screen speaks in plain words about photos, devices and safety, never about partitions, services or certificates. Technical detail is available behind an explicit "Technical details" control.
- **One UI for every screen.** A single web UI codebase (`ui/`), served by `alpd`, is used by the local console, by a laptop browser on the home network, and by the phone app. Layouts differ (large screen and phone), but the API, the data, the wording and the error codes are identical, so the code is shared. VERIFY in an ADR how the phone app reuses it when the mobile phase starts.
- **Numbered error codes.** Every problem the user can see has a stable code of the form `ALP-<AREA>-<NNN>` (areas: `INST`, `BOOT`, `UPD`, `STO`, `KEY`, `NET`, `PAIR`, `APP`, `PWR`, `HW`). One source file (`errors/catalog.toml`) defines for each code: severity, a plain-language title, what the user should do, and technician detail with the logs to check. It generates `docs/errors.md` and the UI strings. Codes are never reused or renumbered. The UI always shows the plain message and action first, then the code with a copy button, so the user can search for it or give it to a technician. The same code is written to the journal (structured field `ALP_ERROR`).
- **Ask nothing the system can find out.** Disks, protection level and capacity are decided automatically. Language defaults to English and can be changed on the first screen. Time zone defaults to UTC and is taken from the first paired phone, changeable in settings (no external geolocation lookup, see rule 9).
- **Translation-ready from day one.** All user-facing text lives in translation files; layouts must work right-to-left even though v1 ships in English.
- **Accessible and touch-friendly:** large targets, readable contrast, keyboard navigation.
