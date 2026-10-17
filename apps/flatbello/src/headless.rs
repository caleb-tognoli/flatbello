//! `flatbello --render-test <model> <out-prefix>`: renders a model without a window, picks the
//! face at the centre, and writes `<prefix>.bmp` and `<prefix>-picked.bmp`. Used to check the
//! renderer and picking on machines without a display.

use flatbello_core::{FaceId, ImportOptions, RenderMesh};

use crate::bmp;
use crate::camera::Camera;
use crate::gpu::Gpu;
use crate::shell::Shell;
use crate::viewport::{self, Viewport};

pub const WIDTH: u32 = 400;
pub const HEIGHT: u32 = 300;

/// Renders `mesh` once, optionally highlighting `selected`, and returns the RGBA image and the
/// face under pixel `pick`.
pub fn render(
    gpu: &Gpu,
    mesh: &RenderMesh,
    camera: Option<Camera>,
    selected: Option<FaceId>,
    pick: (u32, u32),
) -> Result<(Vec<u8>, Option<FaceId>), String> {
    let mut viewport = Viewport::new(&gpu.device);
    let home = viewport.set_mesh(&gpu.device, &gpu.queue, mesh);
    viewport.set_face_states(&gpu.queue, |f| if Some(f) == selected { 2 } else { 0 });
    viewport.resize(&gpu.device, WIDTH, HEIGHT);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    viewport.render(
        &gpu.queue,
        &mut encoder,
        &camera.unwrap_or(home),
        Some(pick),
    );
    gpu.queue.submit([encoder.finish()]);
    let picked = viewport.read_pick(&gpu.device);
    let image = viewport
        .read_color(&gpu.device, &gpu.queue)
        .ok_or("could not read the rendered image")?;
    Ok((image, picked))
}

pub fn run(model_path: &str, prefix: &str) -> Result<(), String> {
    let bytes = std::fs::read(model_path).map_err(|e| format!("reading {model_path}: {e}"))?;
    let model = flatbello_engine::import(&bytes, model_path, &ImportOptions::default())
        .map_err(|e| e.to_string())?;
    let gpu = Gpu::headless()?;
    println!("adapter: {}", gpu.adapter.get_info().name);
    let centre = (WIDTH / 2, HEIGHT / 2);
    let (image, picked) = render(&gpu, &model.mesh, None, None, centre)?;
    write(&format!("{prefix}.bmp"), &image)?;
    match picked {
        Some(f) => {
            println!("face under the centre: {}", f.0);
            let (image, _) = render(&gpu, &model.mesh, None, Some(f), centre)?;
            write(&format!("{prefix}-picked.bmp"), &image)?;
        }
        None => println!("no face under the centre"),
    }
    Ok(())
}

fn write(path: &str, rgba: &[u8]) -> Result<(), String> {
    std::fs::write(path, bmp::encode(WIDTH, HEIGHT, rgba))
        .map_err(|e| format!("writing {path}: {e}"))?;
    println!("wrote {path}");
    Ok(())
}

/// `flatbello --ui-snapshot <model> <out.bmp> [face ...]`: renders the whole window (UI, 3D view
/// and preview) without a display, with the given faces selected.
pub fn ui_snapshot(model_path: &str, out: &str, faces: &[String]) -> Result<(), String> {
    const SIZE: [u32; 2] = [1400, 860];
    let gpu = Gpu::headless()?;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: viewport::COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    // Time advances between frames so that egui's fade-in animations finish.
    let input = |frame: u32| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        )),
        time: Some(f64::from(frame) * 0.5),
        ..Default::default()
    };

    let mut shell = Shell::new(&gpu, viewport::COLOR_FORMAT);
    shell.app.open_path(model_path.into());
    while shell.app.is_busy() {
        std::thread::sleep(std::time::Duration::from_millis(10));
        shell.app.poll_loading();
    }
    let ids = faces
        .iter()
        .map(|f| {
            f.parse()
                .map(FaceId)
                .map_err(|_| format!("bad face number \"{f}\""))
        })
        .collect::<Result<Vec<_>, _>>()?;
    shell.app.select_faces(&ids);
    // egui settles its layout over a couple of frames.
    for frame in 0..4 {
        shell.frame(&gpu, input(frame), &view, SIZE);
    }
    let rgba = viewport::read_texture(&gpu.device, &gpu.queue, &target, SIZE[0], SIZE[1])
        .ok_or("could not read the snapshot")?;
    std::fs::write(out, bmp::encode_rle8(SIZE[0], SIZE[1], &rgba))
        .map_err(|e| format!("writing {out}: {e}"))?;
    println!("wrote {out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbello_core::FaceKind;

    /// Rendering and picking on whatever adapter exists (lavapipe in CI). Skipped, with a note,
    /// on machines without any.
    #[test]
    fn picks_the_face_under_the_centre() {
        let Ok(gpu) = Gpu::headless() else {
            eprintln!("no graphics adapter; skipping");
            return;
        };
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/generated/plate_holes.step");
        let bytes = std::fs::read(path).unwrap();
        let model = flatbello_engine::import(&bytes, "plate_holes.step", &ImportOptions::default())
            .unwrap();
        let (image, picked) =
            render(&gpu, &model.mesh, None, None, (WIDTH / 2, HEIGHT / 2)).unwrap();
        assert_eq!(image.len(), (WIDTH * HEIGHT * 4) as usize);
        let face = picked.expect("a face under the centre");
        // The default view looks down at the plate, so the centre is on its top face.
        let normal = model.face_normal(face).unwrap();
        assert!(normal.z > 0.99, "{normal:?}");
        assert_eq!(model.faces[face.0 as usize].kind, FaceKind::Planar);
        // A corner pixel is background.
        let (_, corner) = render(&gpu, &model.mesh, None, None, (0, 0)).unwrap();
        assert_eq!(corner, None);
    }
}
