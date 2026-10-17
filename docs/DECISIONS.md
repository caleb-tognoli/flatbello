# Decisions

Product and architecture decisions, newest last. Each entry: date, decision, reason.

## 2026-10-17 — Development OS is Windows 11; CI is Linux-only until M5

Caleb develops on Windows 11. CI runs on Linux for now; Windows and macOS CI arrive in M5.

## 2026-10-17 — MSVC Rust toolchain on Windows

Windows builds use `stable-x86_64-pc-windows-msvc`. `rust-toolchain.toml` pins only the `stable` channel so it
works on every OS; on Windows the host is chosen with `rustup override set stable-x86_64-pc-windows-msvc`.
Reason: OCCT's Windows binaries (vcpkg, official installers) are MSVC builds and cannot be linked with the GNU
toolchain.

## 2026-10-17 — OCCT on Windows comes from vcpkg

OCCT is installed with `vcpkg install opencascade:x64-windows` (8.0.1 at the time of writing). The exact
version is recorded when it is installed in M2. Reason: it is the documented install path, and it gives an
MSVC build with the headers and import libraries in one place.

## 2026-10-17 — Flatbello's license is deferred

No license is chosen yet. Crates set `publish = false` and have no `license` field, and cargo-deny ignores
private crates. Reason: Caleb deferred the choice. Dependencies are still restricted by `deny.toml`
(no GPL/AGPL), which keeps every option open.

## 2026-10-17 — Agent instructions stay private in `.agents/`

The agent instructions and project plan stay in `.agents/README.md` and `.agents/PLAN.md`, which are
git-ignored. Where they mention `CLAUDE.md` and `docs/PLAN.md`, they mean these files. Reason: Caleb's choice.

## 2026-10-17 — `flatbello-occt` is a pure stub until M2

In M0 the crate has no `build.rs` and does not link OCCT. Reason: CI then needs no OCCT install until the shim
exists.

## 2026-10-17 — Lint policy

- `unsafe_code = "forbid"` in every crate except `flatbello-occt`, which denies
  `clippy::undocumented_unsafe_blocks` (every `unsafe` block needs a `// SAFETY:` comment) and
  `unsafe_op_in_unsafe_fn`.
- `clippy::unwrap_used`, `expect_used` and `panic` warn everywhere; `clippy.toml` allows them in tests. Main
  setup code that needs them uses a local `#[allow]`.
- `clippy::float_cmp` warns, to back up the rule against comparing floats with `==`.
- CI runs clippy with `-D warnings`, so warnings fail the build.

Reason: it enforces the code rules mechanically instead of only through review.

## 2026-10-17 — LF line endings in the repository

`.gitattributes` forces LF for text files and marks fixture formats as binary. Reason: output must be
byte-identical, so golden files must not change line endings depending on the OS or the `core.autocrlf`
setting.
