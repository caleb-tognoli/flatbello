//! One UI frame: runs the egui UI, renders the 3D view it asked for, draws everything into a
//! target texture and reads back the picked face. Shared by the window and `--ui-snapshot`.

use flatbello_core::FaceId;

use crate::app::App;
use crate::gpu::Gpu;
use crate::viewport::Viewport;

pub struct Shell {
    pub ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
    viewport: Viewport,
    view_texture: Option<egui::TextureId>,
    pub app: App,
}

pub struct FrameOutput {
    pub platform: egui::PlatformOutput,
    /// egui wants another frame right away (an animation or a pending state change).
    pub repaint: bool,
}

impl Shell {
    pub fn new(gpu: &Gpu, target_format: wgpu::TextureFormat, app: App) -> Shell {
        Shell {
            ctx: egui::Context::default(),
            renderer: egui_wgpu::Renderer::new(
                &gpu.device,
                target_format,
                egui_wgpu::RendererOptions::default(),
            ),
            viewport: Viewport::new(&gpu.device),
            view_texture: None,
            app,
        }
    }

    /// Runs one frame and draws it into `target`, which is `size` physical pixels.
    pub fn frame(
        &mut self,
        gpu: &Gpu,
        input: egui::RawInput,
        target: &wgpu::TextureView,
        size: [u32; 2],
    ) -> FrameOutput {
        let (device, queue) = (&gpu.device, &gpu.queue);
        self.app.poll_loading();
        if std::mem::take(&mut self.app.new_mesh) {
            let empty = flatbello_core::RenderMesh::default();
            let mesh = self.app.mesh().unwrap_or(&empty);
            self.app.camera = self.viewport.set_mesh(device, queue, mesh);
        }
        if std::mem::take(&mut self.app.reset_camera)
            && let Some(c) = self.viewport.home_camera()
        {
            self.app.camera = c;
        }

        let texture = self.view_texture;
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui, texture));

        let (w, h) = self.app.view.size;
        if self.viewport.resize(device, w, h)
            && let Some(t) = self.viewport.targets()
        {
            let filter = wgpu::FilterMode::Linear;
            match self.view_texture {
                Some(id) => self.renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &t.color_view,
                    filter,
                    id,
                ),
                None => {
                    self.view_texture = Some(self.renderer.register_native_texture(
                        device,
                        &t.color_view,
                        filter,
                    ));
                }
            }
        }
        if std::mem::take(&mut self.app.states_dirty) {
            let app = &self.app;
            self.viewport.set_face_states(queue, |f| app.face_state(f));
        }

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: output.pixels_per_point,
        };
        let jobs = self
            .ctx
            .tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        // egui asserts that every texture delta is consumed, so drain them.
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in &deltas {
                self.renderer.update_texture(device, queue, id, delta);
            }
        }

        let mut encoder = device.create_command_encoder(&Default::default());
        self.viewport
            .render(queue, &mut encoder, &self.app.camera, self.app.view.pick);
        let extra = self
            .renderer
            .update_buffers(device, queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        queue.submit(extra.into_iter().chain([encoder.finish()]));
        for id in output.textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }

        if self.app.view.pick.is_some() {
            let picked: Option<FaceId> = self.viewport.read_pick(device);
            self.app.set_hovered(picked);
        }
        let egui_repaint = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero());
        FrameOutput {
            platform: std::mem::take(&mut output.platform_output),
            repaint: egui_repaint || self.app.states_dirty || self.app.is_busy(),
        }
    }
}
