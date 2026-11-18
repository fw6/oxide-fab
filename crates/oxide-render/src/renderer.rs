//! The wgpu renderer: one offscreen colour target, one depth buffer, a mesh
//! pipeline and a line pipeline, read back once per frame.
//!
//! Rendering is synchronous and writes into a texture the caller never sees.
//! That is deliberate: the same call paints the CI screenshot and the frame a
//! native component shows, so what a test proves is what the app displays. The
//! UI renders on its own thread and hands the newest frame to the view.

use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use oxide_scene::{Camera, SceneGraph, Shape};

use crate::RenderError;
use crate::glb::Model;
use crate::gpu::{AdapterReport, Gpu};

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const MAX_OBJECTS: usize = 1024;
const OBJECT_ALIGN: u64 = 256;
const OBJECT_BYTES: u64 = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererOptions {
    /// Ask for a CPU adapter first (CI: lavapipe, WARP).
    pub prefer_software: bool,
    /// The largest side a frame may have; larger views render at this size.
    pub max_side: u32,
}

impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            prefer_software: std::env::var("OXIDE_RENDER_SOFTWARE").is_ok_and(|value| value == "1"),
            max_side: 1600,
        }
    }
}

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8, top row first.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

impl Frame {
    pub fn to_png(&self, path: &std::path::Path) -> Result<(), RenderError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        image::save_buffer(
            path,
            &self.rgba,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )
        .map_err(|error| RenderError::Image(error.to_string()))
    }

    /// The average colour of the frame, used by tests to prove that something
    /// was drawn rather than a uniform clear.
    #[must_use]
    pub fn mean_rgb(&self) -> [f32; 3] {
        let mut sum = [0.0_f64; 3];
        let pixels = self.rgba.len() / 4;
        if pixels == 0 {
            return [0.0; 3];
        }
        for pixel in self.rgba.as_chunks::<4>().0 {
            for channel in 0..3 {
                sum[channel] += f64::from(pixel[channel]);
            }
        }
        [
            (sum[0] / pixels as f64 / 255.0) as f32,
            (sum[1] / pixels as f64 / 255.0) as f32,
            (sum[2] / pixels as f64 / 255.0) as f32,
        ]
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
    ambient: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LineVertex {
    position: [f32; 3],
    color: [f32; 4],
}

#[derive(Debug)]
struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    base_color: [f32; 4],
}

#[derive(Debug)]
struct Target {
    texture: wgpu::Texture,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_row: u32,
    width: u32,
    height: u32,
}

pub struct Renderer {
    gpu: Gpu,
    model: Model,
    camera_buffer: wgpu::Buffer,
    camera_bind: wgpu::BindGroup,
    object_buffer: wgpu::Buffer,
    object_bind: wgpu::BindGroup,
    meshes: BTreeMap<String, GpuMesh>,
    cube: GpuMesh,
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    line_buffer: wgpu::Buffer,
    line_capacity: u64,
    target: Option<Target>,
    size: (u32, u32),
    max_side: u32,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("adapter", &self.gpu.adapter)
            .field("size", &self.size)
            .field("meshes", &self.meshes.len())
            .finish_non_exhaustive()
    }
}

impl Renderer {
    #[expect(
        clippy::too_many_lines,
        reason = "one setup: layouts, buffers, pipelines and the mesh upload, in the order the GPU needs them"
    )]
    pub fn new(model: Model, options: RendererOptions) -> Result<Self, RenderError> {
        let gpu = Gpu::new(options.prefer_software)?;
        let camera_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("oxide-render camera layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<CameraUniform>() as u64,
                        ),
                    },
                    count: None,
                }],
            });
        let object_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("oxide-render object layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(OBJECT_BYTES),
                    },
                    count: None,
                }],
            });
        let camera_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oxide-render camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oxide-render camera bind"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let object_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oxide-render objects"),
            size: OBJECT_ALIGN * MAX_OBJECTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let object_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oxide-render object bind"),
            layout: &object_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &object_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(OBJECT_BYTES),
                }),
            }],
        });

        let scene_shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("oxide-render scene"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene.wgsl").into()),
            });
        let line_shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("oxide-render lines"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lines.wgsl").into()),
            });
        let mesh_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("oxide-render mesh layout"),
                bind_group_layouts: &[Some(&camera_layout), Some(&object_layout)],
                immediate_size: 0,
            });
        let line_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("oxide-render line layout"),
                bind_group_layouts: &[Some(&camera_layout)],
                immediate_size: 0,
            });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::glb::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
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
            ],
        };
        let line_layout_vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 12,
                    shader_location: 1,
                },
            ],
        };
        let depth = Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        let mesh_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("oxide-render mesh"),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: &scene_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[vertex_layout],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    // Exploded parts and the floor slab are seen from both
                    // sides; culling would punch holes in them.
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: depth.clone(),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &scene_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let line_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("oxide-render lines"),
                layout: Some(&line_layout),
                vertex: wgpu::VertexState {
                    module: &line_shader,
                    entry_point: Some("vs_line"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[line_layout_vertex],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    // Lines sit on surfaces (the grid on the floor, the AMHS
                    // line on the rails) and must not z-fight them.
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &line_shader,
                    entry_point: Some("fs_line"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });

        let mut meshes = BTreeMap::new();
        for (name, mesh) in model.meshes() {
            meshes.insert(name.to_owned(), upload_mesh(&gpu, name, mesh));
        }
        let cube = upload_mesh(&gpu, "unit cube", &unit_cube());
        let line_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oxide-render lines"),
            size: 4096 * std::mem::size_of::<LineVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            gpu,
            model,
            camera_buffer,
            camera_bind,
            object_buffer,
            object_bind,
            meshes,
            cube,
            mesh_pipeline,
            line_pipeline,
            line_buffer,
            line_capacity: 4096,
            target: None,
            size: (1280, 800),
            max_side: options.max_side.max(320),
        })
    }

    #[must_use]
    pub fn adapter(&self) -> &AdapterReport {
        &self.gpu.adapter
    }

    #[must_use]
    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.clamp(64, self.max_side);
        let height = height.clamp(64, self.max_side);
        self.size = (width, height);
    }

    /// Draw one frame and read it back.
    #[expect(
        clippy::too_many_lines,
        reason = "the frame: uniforms, draw list, lines, pass, readback; splitting it would hide the ordering the GPU requires"
    )]
    pub fn render(&mut self, graph: &SceneGraph, camera: &Camera) -> Result<Frame, RenderError> {
        let (width, height) = self.size;
        if self
            .target
            .as_ref()
            .is_none_or(|target| target.width != width || target.height != height)
        {
            self.target = Some(self.create_target(width, height));
        }
        let aspect = width as f32 / height as f32;
        let eye = camera.eye();
        let camera_uniform = CameraUniform {
            view_proj: camera.view_projection(aspect).to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 1.0],
            light: [-0.42, 0.86, 0.30, 0.0],
            ambient: [0.56, 0.0, 0.0, 0.0],
        };
        self.gpu
            .queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));

        let line_vertices = line_vertices(graph);
        self.ensure_line_capacity(line_vertices.len() as u64);
        if !line_vertices.is_empty() {
            self.gpu
                .queue
                .write_buffer(&self.line_buffer, 0, bytemuck::cast_slice(&line_vertices));
        }
        let mut draws: Vec<(&GpuMesh, u32)> = Vec::with_capacity(graph.nodes.len());
        for (index, node) in graph.nodes.iter().enumerate() {
            if index >= MAX_OBJECTS {
                log::warn!("renderer: more than {MAX_OBJECTS} nodes, the rest are not drawn");
                break;
            }
            let mesh = match (&node.mesh, node.shape) {
                (Some(name), _) => match self.meshes.get(name) {
                    Some(mesh) => mesh,
                    None => continue,
                },
                (None, Some(Shape::Cube)) => &self.cube,
                (None, None) => continue,
            };
            let color = [
                mesh.base_color[0] * node.color[0],
                mesh.base_color[1] * node.color[1],
                mesh.base_color[2] * node.color[2],
                node.color[3],
            ];
            let object = ObjectUniform {
                model: node.transform.to_cols_array_2d(),
                color,
                params: [node.emissive, if node.dimmed { 1.0 } else { 0.0 }, 0.0, 0.0],
            };
            let offset = index as u64 * OBJECT_ALIGN;
            self.gpu
                .queue
                .write_buffer(&self.object_buffer, offset, bytemuck::bytes_of(&object));
            draws.push((mesh, offset as u32));
        }

        let target = self.target.as_ref().expect("just created");

        let [r, g, b, a] = graph.background;
        let clear = wgpu::Color {
            r: linear(r),
            g: linear(g),
            b: linear(b),
            a: f64::from(a),
        };
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("oxide-render"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("oxide-render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
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
            pass.set_bind_group(0, &self.camera_bind, &[]);
            pass.set_pipeline(&self.mesh_pipeline);
            for (mesh, offset) in &draws {
                pass.set_bind_group(1, &self.object_bind, &[*offset]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            if !line_vertices.is_empty() {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_bind_group(0, &self.camera_bind, &[]);
                pass.set_vertex_buffer(0, self.line_buffer.slice(..));
                pass.draw(0..line_vertices.len() as u32, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);

        let slice = target.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|error| RenderError::Device(error.to_string()))?;
        let view = slice.get_mapped_range();
        let row_bytes = (width * 4) as usize;
        let mut rgba = vec![0_u8; row_bytes * height as usize];
        for row in 0..height as usize {
            let start = row * target.padded_row as usize;
            rgba[row * row_bytes..(row + 1) * row_bytes]
                .copy_from_slice(&view[start..start + row_bytes]);
        }
        drop(view);
        target.readback.unmap();

        Ok(Frame {
            width,
            height,
            rgba,
        })
    }

    fn ensure_line_capacity(&mut self, needed: u64) {
        if needed <= self.line_capacity {
            return;
        }
        let capacity = needed.next_power_of_two();
        self.line_buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oxide-render lines"),
            size: capacity * std::mem::size_of::<LineVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.line_capacity = capacity;
    }

    fn create_target(&self, width: u32, height: u32) -> Target {
        let color_texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("oxide-render colour"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("oxide-render depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_row = (width * 4).div_ceil(256) * 256;
        let readback = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oxide-render readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Target {
            texture: color_texture,
            color,
            depth,
            readback,
            padded_row,
            width,
            height,
        }
    }
}

fn upload_mesh(gpu: &Gpu, label: &str, mesh: &crate::glb::Mesh) -> GpuMesh {
    use wgpu::util::DeviceExt as _;
    GpuMesh {
        vertices: gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        indices: gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        index_count: mesh.indices.len() as u32,
        base_color: mesh.base_color,
    }
}

fn unit_cube() -> crate::glb::Mesh {
    use crate::glb::Vertex;
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 0.0, 1.0],
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [0.5, -0.5, 0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
                [-0.5, -0.5, 0.5],
            ],
        ),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for position in corners {
            vertices.push(Vertex { position, normal });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    crate::glb::Mesh {
        vertices,
        indices,
        base_color: [1.0, 1.0, 1.0, 1.0],
    }
}

fn line_vertices(graph: &SceneGraph) -> Vec<LineVertex> {
    let mut vertices = Vec::with_capacity(graph.lines.len() * 2);
    for line in &graph.lines {
        vertices.push(LineVertex {
            position: line.a.to_array(),
            color: line.color,
        });
        vertices.push(LineVertex {
            position: line.b.to_array(),
            color: line.color,
        });
    }
    vertices
}

fn linear(channel: f32) -> f64 {
    let channel = f64::from(channel.clamp(0.0, 1.0));
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}
