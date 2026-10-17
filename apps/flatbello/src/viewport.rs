//! Renders the model into offscreen textures: a shaded colour image shown through egui, and an
//! `R32Uint` id image where each pixel holds `face id + 1` (0 is background) for picking.

use std::sync::mpsc;

use flatbello_core::{FaceId, Point3, RenderMesh, Vec3};

use crate::camera::{Camera, to_f32};

/// The colour target format. egui samples it as a plain (gamma-space) texture.
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.16,
    g: 0.17,
    b: 0.19,
    a: 1.0,
};

/// Per-vertex attributes in buffer slot 0.
const VERTEX_STRIDE: u64 = 28;
/// Readback rows must be padded to this many bytes.
const ROW_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

pub struct Targets {
    pub width: u32,
    pub height: u32,
    pub color: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    id: wgpu::Texture,
    id_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

struct Mesh {
    vertices: wgpu::Buffer,
    states: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// The face of every vertex, to rebuild the state buffer.
    face_of_vertex: Vec<u32>,
    face_count: usize,
    /// Each face's vertex range, when every face's vertices are contiguous (importers emit
    /// them that way); lets a hover change rewrite only the faces that changed.
    face_ranges: Option<Vec<std::ops::Range<u32>>>,
    /// The state last uploaded for each face.
    uploaded: std::cell::RefCell<Vec<u32>>,
    /// The bounding-box centre. Positions are uploaded relative to it to keep f32 precision.
    center: Point3,
    radius: f64,
}

pub struct Viewport {
    color_pipeline: wgpu::RenderPipeline,
    id_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    readback: wgpu::Buffer,
    mesh: Option<Mesh>,
    targets: Option<Targets>,
    pick_pending: bool,
}

impl Viewport {
    pub fn new(device: &wgpu::Device) -> Viewport {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("viewport.wgsl").into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport uniforms"),
            size: 80,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 24,
                shader_location: 2,
            },
        ];
        let state_attribute = [wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: 0,
            shader_location: 3,
        }];
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 4,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &state_attribute,
            }),
        ];
        let pipeline = |entry: &str, format: wgpu::TextureFormat, label: &str| {
            let targets = [Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let color_pipeline = pipeline("fs_color", COLOR_FORMAT, "viewport colour");
        let id_pipeline = pipeline("fs_id", ID_FORMAT, "viewport ids");
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick readback"),
            size: u64::from(ROW_ALIGN),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Viewport {
            color_pipeline,
            id_pipeline,
            uniforms,
            bind_group,
            readback,
            mesh: None,
            targets: None,
            pick_pending: false,
        }
    }

    /// Uploads a model's render mesh. Returns the camera that frames it.
    pub fn set_mesh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &RenderMesh,
    ) -> Camera {
        let positions = mesh.positions();
        let (mut lo, mut hi) = (
            Point3::new(f64::MAX, f64::MAX, f64::MAX),
            Point3::new(f64::MIN, f64::MIN, f64::MIN),
        );
        for p in positions {
            lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        let (center, radius) = if positions.is_empty() {
            (Point3::ORIGIN, 1.0)
        } else {
            let c = Point3::new(
                0.5 * (lo.x + hi.x),
                0.5 * (lo.y + hi.y),
                0.5 * (lo.z + hi.z),
            );
            (c, 0.5 * lo.distance(hi))
        };

        let mut bytes = Vec::with_capacity(positions.len() * VERTEX_STRIDE as usize);
        for ((p, n), f) in positions.iter().zip(mesh.normals()).zip(mesh.face_ids()) {
            let r = *p - center;
            for v in [r.x, r.y, r.z, n.x, n.y, n.z] {
                bytes.extend_from_slice(&(v as f32).to_le_bytes());
            }
            bytes.extend_from_slice(&f.0.to_le_bytes());
        }
        let index_bytes: Vec<u8> = mesh
            .indices()
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        let face_of_vertex: Vec<u32> = mesh.face_ids().iter().map(|f| f.0).collect();
        let face_count = face_of_vertex.iter().max().map_or(0, |m| *m as usize + 1);
        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                // Zero-sized buffers are not allowed; pad empty meshes to 4 bytes.
                size: (contents.len().max(4) as u64).next_multiple_of(4),
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if !contents.is_empty() {
                queue.write_buffer(&b, 0, contents);
            }
            b
        };
        let vertices = buffer("vertices", &bytes, wgpu::BufferUsages::VERTEX);
        let indices = buffer("indices", &index_bytes, wgpu::BufferUsages::INDEX);
        let states = buffer(
            "face states",
            &vec![0u8; face_of_vertex.len() * 4],
            wgpu::BufferUsages::VERTEX,
        );
        self.mesh = Some(Mesh {
            vertices,
            states,
            indices,
            index_count: u32::try_from(mesh.indices().len()).unwrap_or(0),
            face_ranges: contiguous_ranges(&face_of_vertex, face_count),
            uploaded: std::cell::RefCell::new(vec![0; face_count]),
            face_of_vertex,
            face_count,
            center,
            radius,
        });
        Camera::framing(center, radius)
    }

    /// Updates the highlight state of every face: bit 0 hovered, bit 1 selected.
    pub fn set_face_states(&self, queue: &wgpu::Queue, state: impl Fn(FaceId) -> u32) {
        let Some(mesh) = &self.mesh else { return };
        let per_face: Vec<u32> = (0..mesh.face_count)
            .map(|f| state(FaceId(f as u32)))
            .collect();
        let mut uploaded = mesh.uploaded.borrow_mut();
        if let Some(ranges) = &mesh.face_ranges {
            for (f, range) in ranges.iter().enumerate() {
                if per_face[f] != uploaded[f] && !range.is_empty() {
                    let bytes: Vec<u8> =
                        std::iter::repeat_n(per_face[f].to_le_bytes(), range.len())
                            .flatten()
                            .collect();
                    queue.write_buffer(&mesh.states, u64::from(range.start) * 4, &bytes);
                }
            }
            *uploaded = per_face;
            return;
        }
        let bytes: Vec<u8> = mesh
            .face_of_vertex
            .iter()
            .flat_map(|f| {
                per_face
                    .get(*f as usize)
                    .copied()
                    .unwrap_or(0)
                    .to_le_bytes()
            })
            .collect();
        if !bytes.is_empty() {
            queue.write_buffer(&mesh.states, 0, &bytes);
        }
        *uploaded = per_face;
    }

    /// The camera that frames the current mesh.
    pub fn home_camera(&self) -> Option<Camera> {
        self.mesh
            .as_ref()
            .map(|m| Camera::framing(m.center, m.radius))
    }

    pub fn targets(&self) -> Option<&Targets> {
        self.targets.as_ref()
    }

    /// Makes the render targets `width` × `height`. Returns true if they were recreated.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if self
            .targets
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height)
        {
            return false;
        }
        let texture = |label: &str, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            "viewport colour",
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let id = texture(
            "viewport ids",
            ID_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            "viewport depth",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        self.targets = Some(Targets {
            width,
            height,
            color_view: view(&color),
            id_view: view(&id),
            depth_view: view(&depth),
            color,
            id,
        });
        true
    }

    /// Records the colour and id passes. If `pick` is a pixel inside the view, also copies its
    /// id for [`Viewport::read_pick`] to fetch after the commands are submitted.
    pub fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera: &Camera,
        pick: Option<(u32, u32)>,
    ) {
        let Some(t) = &self.targets else { return };
        let origin = self.mesh.as_ref().map_or(Point3::ORIGIN, |m| m.center);
        let aspect = f64::from(t.width) / f64::from(t.height);
        let view_proj = camera.projection(aspect) * camera.view(origin);
        let mut data = Vec::with_capacity(80);
        for col in to_f32(&view_proj) {
            for v in col {
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
        let l: Vec3 = camera.light_dir();
        for v in [l.x as f32, l.y as f32, l.z as f32, 0.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &data);

        for (view, pipeline, clear) in [
            (&t.color_view, &self.color_pipeline, BACKGROUND),
            (&t.id_view, &self.id_pipeline, wgpu::Color::TRANSPARENT),
        ] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(m) = &self.mesh {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, m.vertices.slice(..));
                pass.set_vertex_buffer(1, m.states.slice(..));
                pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.index_count, 0, 0..1);
            }
        }

        self.pick_pending = false;
        if let Some((x, y)) = pick.filter(|&(x, y)| x < t.width && y < t.height) {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &t.id,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(ROW_ALIGN),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            self.pick_pending = true;
        }
    }

    /// The face under the pick pixel of the last submitted [`Viewport::render`], if any.
    /// Blocks until the GPU has finished that frame.
    pub fn read_pick(&mut self, device: &wgpu::Device) -> Option<FaceId> {
        if !std::mem::take(&mut self.pick_pending) {
            return None;
        }
        let raw = read_buffer(device, &self.readback, 4)?;
        let id = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        id.checked_sub(1).map(FaceId)
    }

    /// Copies the colour image into tightly packed RGBA8 rows, top row first. Headless use only.
    pub fn read_color(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Vec<u8>> {
        let t = self.targets.as_ref()?;
        read_texture(device, queue, &t.color, t.width, t.height)
    }
}

/// Copies an RGBA8 texture with COPY_SRC usage into tightly packed rows, top row first.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let row = (width * 4).next_multiple_of(ROW_ALIGN);
    let size = u64::from(row) * u64::from(height);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let padded = read_buffer(device, &buffer, size)?;
    let tight = (width * 4) as usize;
    let mut out = Vec::with_capacity(tight * height as usize);
    for r in padded.chunks(row as usize) {
        out.extend_from_slice(&r[..tight]);
    }
    Some(out)
}

/// Maps `len` bytes of a MAP_READ buffer, waiting for the GPU, and copies them out.
fn read_buffer(device: &wgpu::Device, buffer: &wgpu::Buffer, len: u64) -> Option<Vec<u8>> {
    let slice = buffer.slice(..len);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok()?;
    rx.recv().ok()?.ok()?;
    let data = slice.get_mapped_range().ok()?.to_vec();
    buffer.unmap();
    Some(data)
}

/// Each face's vertex range if every face's vertices form one contiguous run.
fn contiguous_ranges(
    face_of_vertex: &[u32],
    face_count: usize,
) -> Option<Vec<std::ops::Range<u32>>> {
    let mut ranges = vec![0..0; face_count];
    let mut seen = vec![false; face_count];
    let mut start = 0;
    for i in 1..=face_of_vertex.len() {
        if i == face_of_vertex.len() || face_of_vertex[i] != face_of_vertex[start] {
            let f = face_of_vertex[start] as usize;
            if std::mem::replace(&mut seen[f], true) {
                return None;
            }
            ranges[f] = u32::try_from(start).ok()?..u32::try_from(i).ok()?;
            start = i;
        }
    }
    Some(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_contiguous_face_ranges() {
        assert_eq!(
            contiguous_ranges(&[0, 0, 1, 1, 1, 2], 3),
            Some(vec![0..2, 2..5, 5..6])
        );
        assert_eq!(contiguous_ranges(&[0, 1, 0], 2), None);
        assert_eq!(contiguous_ranges(&[], 0), Some(vec![]));
        // A face without vertices keeps an empty range.
        assert_eq!(contiguous_ranges(&[0, 2], 3), Some(vec![0..1, 0..0, 1..2]));
    }
}
