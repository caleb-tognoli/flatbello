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

## 2026-10-17 — Core type refinements (M1)

- `Segment::Circle` and `Segment::Ellipse` carry a `ccw` flag. A loop made of one circle needs a
  direction, so that holes can wind clockwise like every other loop.
- `Segment::Spline` wraps a `Spline` struct (degree, knots, control points, optional weights), so
  that evaluation, reversal and validation live in one place.
- An `Edge` pairs a `Segment` with its `Provenance`; a `Loop` is a validated list of edges.
- `Loop` and `Profile2D` have private fields and validating constructors. `Profile2D::new` orients
  the outer loop counter-clockwise and the holes clockwise, and rejects holes that lie outside the
  outer loop or inside another hole.
- Translating a profile moves its `Frame3` origin too, so that profile coordinates always equal
  `frame.to_local_2d(world point)`.
- Geometry types do not implement `PartialEq`; tests and code compare through `Tolerance`.
- `SelectableModel` has no handle to the importer's geometry yet. That handle is designed with
  the STEP importer in M2.

Reason: these are the smallest changes that make the planned types well-defined and keep their
invariants.

## 2026-10-17 — Exporters are identified by id, one per format version

`Exporter` gains `id()` (for example `dxf-r12` or `svg`), and each DXF version is its own exporter.
`ExportOptions` holds only shared settings: currently the tolerance used for flattening. Writers
write profiles where they are; `layout_side_by_side` and `Profile2D::at_origin` position them.
Reason: this avoids format-specific fields in a shared options struct, and the UI can list
exporters directly.

## 2026-10-17 — Writer output conventions

- Numbers are rounded to 6 decimals, trailing zeros are trimmed but one decimal is kept
  (`2.0`), and negative zero is never written. This keeps output byte-identical across runs.
- DXF uses CRLF line ends and right-aligned group codes, as AutoCAD writes them. Layer colours:
  `CUT_OUTER` 7, `CUT_INNER` 1 (red), so cutter software can map them to different settings.
- DXF R12 has no `ELLIPSE` or `SPLINE` entity, so ellipses, splines and polyline fallbacks become
  `POLYLINE`s flattened within `tolerance.fit()` (0.01 mm by default). Bulges are not used yet.
- SVG: the drawing is moved so that its bounding box starts at (0, 0) and y is flipped; one user
  unit is 1 mm. The stroke is 0.0254 mm (0.001 in hairline), with no fill and one `<path>` per loop.
  Ellipses are written as exact `A` commands and splines are flattened.

Reason: deterministic, cutter-safe output that common tools read.

## 2026-10-17 — Golden files are blessed explicitly

Golden outputs live in `tests/fixtures/golden/`. Running the tests with `FLATBELLO_BLESS=1`
rewrites them; every rewrite is reviewed (and the SVG checked visually) before it is committed.
The hand-built sample profiles are in `flatbello_core::sample`.

## 2026-10-17 — Linux container for OCCT development (M2)

Development builds that need OCCT run in the Docker image in `tools/docker` (Ubuntu 26.04, OCCT 7.9.2).
CI runs the same checks in Ubuntu 26.04 and 24.04 containers, covering OCCT 7.9 and 7.6 and both
toolkit naming schemes. Reason: Caleb's choice; the Windows machine has no C++ toolchain yet, and the
container matches CI exactly. Native Windows builds (vcpkg) follow once the C++ workload is installed.

## 2026-10-17 — B-rep data is copied out of OCCT at import

The STEP/IGES importer copies each planar face's plane and wires (lines, circles, ellipses, trimmed
B-splines, or sampled points for any other curve) into plain types in `flatbello_core::brep`.
`SelectableModel::geometry` holds them, and `flatbello-extract` works on them in pure Rust. No OCCT
handle outlives the import. Reason: extraction stays testable without OCCT, the worker-process
split in M5 only has to send plain data, and OCCT stays inside two crates.

## 2026-10-17 — Shim behaviour (M2)

- STEP is read from memory (`ReadStream`, OCCT 7.7 and later). IGES, and STEP on older OCCT, go
  through a temporary file, because those readers only read files. The file is deleted afterwards.
- One global lock serialises every STEP/IGES read and write: OCCT's translators share global state
  and crashed when files were loaded on several threads at once.
- OCCT's default messenger is silenced; failures reach Rust through `fb_last_error()`.
- Plane normals are computed as X × Y of the plane placement (correct for left-handed placements),
  then flipped for reversed faces, so they point out of the solid.
- B-splines are made non-periodic and trimmed to the edge's range in the shim, so Rust receives
  clamped knot vectors.
- IGES units come from the global section's unit flag. `UnitValue()` returns cascade units, not metres
  as its documentation says.

## 2026-10-17 — Extraction details (M2)

- Edge directions are repaired from geometry when the orientation flags do not chain.
- Edges shorter than the linear tolerance are dropped; the gap they leave is within tolerance.
- When several straight edges tie for longest (within the linear tolerance), the first in loop order
  sets the x axis.
- Every planar face's profile is checked against ground truth, not byte-for-byte golden files: edge order
  and start points can differ between OCCT versions, while the geometry must not. Tests also check that
  every frame is right-handed with z along the outward normal, which rules out mirrored output.

## 2026-10-17 — Desktop app structure (M3)

- Versions: egui, egui-wgpu and egui-winit 0.37, wgpu 30, winit 0.30, rfd 0.17, all with default
  features off. eframe is not used; the app drives winit, egui-winit and egui-wgpu itself.
- The 3D view renders into an offscreen texture that egui shows as an image, instead of a paint
  callback. The scene needs its own depth buffer and id target, and the same path renders headless
  (`--render-test`, `--ui-snapshot`, and the picking test on lavapipe in CI).
- Picking renders `face id + 1` into an `R32Uint` target and reads back the pixel under the cursor
  each frame the pointer is over the view.
- Face highlight state (hovered, selected) is a per-vertex attribute re-uploaded when it changes, so
  no storage buffers are needed and the GL fallback backend works.
- wgpu's async setup calls run on a hand-written `block_on`; there is no async runtime.
- Only flat faces can be selected for export; clicking a curved face says so in the status bar.
- The app and the CLI both export through `Engine::export_faces`, so the same faces give identical
  files.

## 2026-10-17 — Transitive thiserror and egui's font licences (pending approval)

- `thiserror` appears only inside wgpu, egui-wgpu and the Wayland stack, never in Flatbello's code.
  `deny.toml` allows it only through those parents.
- egui's bundled fonts (`epaint_default_fonts`) are under OFL-1.1 and the Ubuntu Font Licence as well as
  MIT/Apache. `deny.toml` allows those two font licences for that crate only. **Caleb to approve**; the
  alternative is bundling a different font under a licence already allowed.
- The cargo-deny graph is limited to desktop targets, so wasm-only dependencies (such as tokio through
  wasm-bindgen-futures) are not counted.

## 2026-10-17 — Mesh input (M4)

- Meshes are welded (1e-7 of the bounding-box diagonal) and split into planar regions when they load.
  Each region is a selectable face, so picking, selection and export work the same as for STEP. Curved
  surfaces become many thin flat faces (one per facet strip); they can be selected but are rarely useful.
- Regions grow from the largest unassigned triangle; a neighbour joins when its normal is within the
  angular tolerance of the refitted plane and its corners within the linear tolerance.
- Fitting: whole loops of at least 6 points become a circle when every point is within the fit
  tolerance. Otherwise runs grow greedily from the sharpest corner (or the start of the longest chord):
  the longer of a line or an arc wins, and ties go to the line. Arcs need at least 4 points, at most 50°
  per chord, one turning direction, and chords within 4× of each other, so a line next to an arc is
  never swallowed by one flat circle.
- Joints are moved onto the arcs that meet there (onto the intersection between two arcs). When two
  fitted circles miss each other, a short line bridges them, so loops always close.
- Without a chosen unit, mesh coordinates are read as millimetres and reported as unknown. The app asks
  and reloads at the chosen scale. Snapping radii to standard hole sizes (an optional step in the plan)
  is not implemented yet.
- Results on the generated fixtures: plate with holes, slotted plate, asymmetric plate and L-bracket
  reproduce the exact profiles at 0.1, 0.01 and 0.002 mm deflection, coarse included (same counts and
  radii). The B-spline part's curve becomes 2–8 arcs plus short lines, within 0.01 mm of the outline.

## 2026-10-17 — DXF R2000 is the default (M5)

`dxf` now means DXF R2000 (AC1015) with `$INSUNITS = 4`; R12 stays available as `dxf-r12`. The writer emits
every table AutoCAD requires, model and paper space blocks and the root dictionary, with unique handles
and valid owners. Two export options, both off by default because many cutters read only lines and arcs:
`native_curves` (ELLIPSE and SPLINE entities instead of polylines) and `loops_as_polylines` (each loop as one
closed LWPOLYLINE with arcs as bulges). Plot-style and layout objects are not written; LibreCAD, QCAD,
Inkscape and LightBurn do not need them. Caleb to confirm in his tools.

## 2026-10-17 — Worker process (M5)

The app runs the engine in a child process of its own executable (`--worker`), over the framed protocol
in `flatbello_engine::wire`: a little-endian u32 length, then a hand-encoded message. A backend thread owns
the worker; if the worker dies, the job fails, the app drops the lost model and says so, and the next job
starts a fresh worker. Render meshes cross the pipe as f32; profiles as f64 and are revalidated.
`FLATBELLO_IN_PROCESS=1` keeps the engine in the app; tests and headless snapshots use that mode. The
`Crash` request exists only so tests can kill the worker on purpose.

## 2026-10-17 — Packaging (M5)

cargo-packager builds the installers in CI. Linux: a `.deb` that depends on Ubuntu 26.04's OCCT 7.9
packages instead of bundling them. Windows: NSIS, with vcpkg's OCCT DLLs and licence next to the
executable. macOS: an `.app` whose Homebrew dylibs are copied into `Contents/Frameworks` and rewritten to
`@rpath`, ad-hoc signed, in a `.dmg`. No app icon yet. Installers are unsigned, so Windows SmartScreen and
macOS Gatekeeper will warn until signing certificates are set up.
