// Release builds on Windows are GUI programs, without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! Flatbello desktop application (egui + wgpu).

mod app;
mod backend;
mod bmp;
mod camera;
mod gpu;
mod headless;
mod preview;
mod shell;
mod viewport;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use app::App;
use backend::Backend;
use gpu::Gpu;
use shell::Shell;

const USAGE: &str = "usage: flatbello [model]
       flatbello --render-test <model> <out-prefix>
       flatbello --ui-snapshot <model> <out.bmp> [face ...]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [flag] if flag == "--worker" => {
            flatbello_engine::wire::serve(std::io::stdin().lock(), std::io::stdout().lock())
                .map_err(|e| format!("worker: {e}"))
        }
        [flag, model, prefix] if flag == "--render-test" => headless::run(model, prefix),
        [flag, model, out, faces @ ..] if flag == "--ui-snapshot" => {
            headless::ui_snapshot(model, out, faces)
        }
        [] => run_window(None),
        [path] if !path.starts_with('-') => run_window(Some(path.into())),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("flatbello: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_window(open: Option<PathBuf>) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("cannot start the event loop: {e}"))?;
    let mut runner = Runner {
        open,
        state: None,
        error: None,
    };
    event_loop
        .run_app(&mut runner)
        .map_err(|e| format!("event loop failed: {e}"))?;
    runner.error.map_or(Ok(()), Err)
}

struct Runner {
    open: Option<PathBuf>,
    state: Option<Running>,
    error: Option<String>,
}

/// Everything that exists while the window is open.
struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    egui_state: egui_winit::State,
    shell: Shell,
}

impl Running {
    fn new(event_loop: &ActiveEventLoop) -> Result<Running, String> {
        let attrs = Window::default_attributes()
            .with_title("Flatbello")
            .with_inner_size(winit::dpi::LogicalSize::new(1400.0, 860.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("cannot open a window: {e}"))?,
        );
        let instance = Gpu::instance_for(Box::new(window.clone()));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot create a drawing surface: {e}"))?;
        let gpu = Gpu::new(instance, Some(&surface))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&gpu.adapter);
        // egui blends in gamma space, so prefer a non-sRGB surface.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("the surface supports no formats")?;
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface is not supported by the adapter")?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&gpu.device, &config);

        // The engine runs in a worker process unless FLATBELLO_IN_PROCESS is set (for debugging).
        let backend = match std::env::current_exe() {
            Ok(exe) if std::env::var_os("FLATBELLO_IN_PROCESS").is_none() => Backend::worker(exe),
            _ => Backend::in_process(),
        };
        let shell = Shell::new(&gpu, format, App::new(backend));
        let egui_state = egui_winit::State::new(
            shell.ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        Ok(Running {
            window,
            surface,
            config,
            gpu,
            egui_state,
            shell,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.gpu.device, &self.config);
        }
    }

    fn redraw(&mut self) {
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                self.window.request_redraw();
                return;
            }
            _ => return,
        };
        let target = surface_texture.texture.create_view(&Default::default());
        let input = self.egui_state.take_egui_input(&self.window);
        let out = self.shell.frame(
            &self.gpu,
            input,
            &target,
            [self.config.width, self.config.height],
        );
        self.window.pre_present_notify();
        self.gpu.queue.present(surface_texture);
        self.egui_state
            .handle_platform_output(&self.window, out.platform);
        if out.repaint {
            self.window.request_redraw();
        }
    }
}

impl ApplicationHandler for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match Running::new(event_loop) {
            Ok(mut s) => {
                if let Some(path) = self.open.take() {
                    s.shell.app.open_path(path);
                }
                s.window.request_redraw();
                self.state = Some(s);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        let response = s.egui_state.on_window_event(&s.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => s.resize(size.width, size.height),
            WindowEvent::DroppedFile(path) => s.shell.app.open_path(path),
            WindowEvent::RedrawRequested => s.redraw(),
            _ => {}
        }
        if response.repaint {
            s.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(s) = &self.state
            && s.shell.app.is_busy()
        {
            s.window.request_redraw();
        }
    }
}
