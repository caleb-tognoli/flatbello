# Flatbello

Flatbello is a desktop application for Windows, macOS and Linux. Open a 3D model, click a face, and export a
clean 2D CAD file of that face as DXF or SVG. The output is ready for a laser, CNC, waterjet or plasma cutter:
true lines, arcs and circles, with holes included and correct units (millimetres).

- **B-rep input (STEP, IGES):** exact curves taken from the model.
- **Mesh input (STL, OBJ):** lines, arcs and circles reconstructed from the facets, instead of faceted
  polylines.

Status: early development. The 2D core and the SVG and DXF R12 writers work; model import is next.

## Building

Requirements:

- Rust, latest stable. `rust-toolchain.toml` selects the stable channel and the `rustfmt` and `clippy`
  components.
- **Windows:** the MSVC toolchain (`stable-x86_64-pc-windows-msvc`) and Visual Studio or the Build Tools with
  the **Desktop development with C++** workload. OCCT's Windows builds are MSVC builds, so the GNU toolchain is
  not supported.
- Open Cascade (OCCT), linked dynamically. It is not needed yet: the OCCT shim arrives in milestone M2.

```sh
cargo build --workspace
cargo test --workspace
```

Try the writers with the built-in demo part (a 100 × 60 mm plate with rounded corners and two
holes):

```sh
cargo run -p flatbello-cli -- demo-export -o out
```

This writes `out/demo.dxf` (DXF R12, millimetres) and `out/demo.svg`. Golden files are in
`tests/fixtures/golden/`; after a deliberate output change, regenerate them with
`FLATBELLO_BLESS=1 cargo test` and review the diff.

Checks run before every commit and in CI:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

License policy is checked with [cargo-deny](https://github.com/EmbarkStudios/cargo-deny), a build-time tool:

```sh
cargo deny check licenses bans sources
```

## Installing OCCT

Flatbello links OCCT dynamically (OCCT is LGPL-2.1 with an exception). Which OCCT versions are supported is
decided in M2; OCCT 7.8 renamed several toolkits, and the build will handle both naming schemes.

| OS | Command | OCCT version (Oct 2026) |
|---|---|---|
| Debian 13 / Ubuntu 25.10+ | `sudo apt install libocct-foundation-dev libocct-modeling-data-dev libocct-modeling-algorithms-dev libocct-data-exchange-dev` | 7.8.1 (Debian 13), 7.9.2 (Ubuntu 26.04) |
| macOS (Homebrew) | `brew install opencascade` | 7.9.3 |
| Windows (vcpkg) | `vcpkg install opencascade:x64-windows` | 8.0.1 |

Ubuntu 24.04 ships OCCT 7.6.3, and Ubuntu 22.04 ships 7.5.1.

If OCCT is not in a standard location, set `OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR` before building.

## Layout

```
crates/flatbello-core          types, math, Tolerance, Importer/Exporter traits (no I/O)
crates/flatbello-extract       planar B-rep extraction, mesh region growing + arc fitting
crates/flatbello-occt          C++ shim (C API) + build.rs + safe Rust wrapper
crates/flatbello-import-step   STEP/IGES importer (uses flatbello-occt)
crates/flatbello-import-mesh   STL / OBJ readers
crates/flatbello-export-dxf    DXF writer (R12 + R2000)
crates/flatbello-export-svg    SVG writer
crates/flatbello-engine        format registry + orchestration
apps/flatbello-cli             CLI: inspect, export, batch, test harness
apps/flatbello                 desktop app (egui + wgpu)
tools/gen-fixtures             generates test models with OCCT
```

Product and architecture decisions are recorded in [docs/DECISIONS.md](docs/DECISIONS.md).

## License

Not yet decided.
