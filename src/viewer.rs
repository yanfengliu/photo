use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use iced::event;
use iced::mouse;
use iced::widget::shader;
use iced::widget::shader::wgpu;
use iced::{Point, Rectangle};

use crate::decode::ImageData;

// ---------------------------------------------------------------------------
// Messages emitted by the viewer shader widget
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ViewerEvent {
    Zoom {
        factor: f32,
        cursor: [f32; 2],
        canvas_size: [f32; 2],
    },
    Pan {
        delta: [f32; 2],
    },
    #[allow(dead_code)]
    DoubleClick {
        canvas_size: [f32; 2],
    },
}

// ---------------------------------------------------------------------------
// Shader program data (recreated each view() call)
// ---------------------------------------------------------------------------

pub struct ImageCanvas {
    pub image: Option<Arc<ImageData>>,
    pub image_id: u64,
    pub zoom: f32,
    pub offset: [f32; 2],
    pub adjustments: AdjustmentUniforms,
}

/// Plain data passed from App to the shader. No GPU types.
#[derive(Debug, Clone, Copy)]
pub struct AdjustmentUniforms {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub temp_matrix: [f32; 9], // row-major 3x3
    pub lens_enabled: bool,
    pub lens_dist: [f32; 3], // a, b, c
    pub lens_vig: [f32; 3],  // k1, k2, k3
    pub lens_tca_r: f32,
    pub lens_tca_b: f32,
    pub image_aspect: f32,
}

impl Default for AdjustmentUniforms {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            temp_matrix: [0.0; 9],
            lens_enabled: false,
            lens_dist: [0.0; 3],
            lens_vig: [0.0; 3],
            lens_tca_r: 0.0,
            lens_tca_b: 0.0,
            image_aspect: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Persistent widget state (managed by iced across view calls)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ViewerState {
    dragging: bool,
    last_pos: Option<Point>,
}

// ---------------------------------------------------------------------------
// Primitive: data sent from draw() to prepare()/render()
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ImagePrimitive {
    image: Option<Arc<ImageData>>,
    image_id: u64,
    /// Image rect in viewport-normalized UV: [left, top, right, bottom]
    rect: [f32; 4],
    adjustments: AdjustmentUniforms,
}

// ---------------------------------------------------------------------------
// GPU uniform buffer layout (must match image.wgsl)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Uniforms {
    rect: [f32; 4],
    bg_color: [f32; 4],
    // Adjustments
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    vibrance: f32,
    saturation: f32,
    clarity: f32,
    dehaze: f32,
    _pad0: f32,
    _pad1: f32,
    // Temperature/tint Bradford matrix (3 rows, padded to vec4 each)
    temp_mat_row0: [f32; 4],
    temp_mat_row1: [f32; 4],
    temp_mat_row2: [f32; 4],
    // Lens corrections
    lens_enabled: f32,
    lens_dist_a: f32,
    lens_dist_b: f32,
    lens_dist_c: f32,
    lens_vig_k1: f32,
    lens_vig_k2: f32,
    lens_vig_k3: f32,
    lens_tca_r_scale: f32,
    lens_tca_b_scale: f32,
    image_aspect: f32,
    _pad2: f32,
    _pad3: f32,
}

// ---------------------------------------------------------------------------
// Cached GPU resources stored in shader::Storage
// ---------------------------------------------------------------------------

struct GpuResources {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    // Per-image state
    texture: Option<wgpu::Texture>,
    texture_view: Option<wgpu::TextureView>,
    bind_group: Option<wgpu::BindGroup>,
    current_image_id: u64,
    blur_texture_view: Option<wgpu::TextureView>,
    // Blur pre-pass pipeline resources
    blur_pipeline: Option<wgpu::RenderPipeline>,
    blur_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blur_uniform_buffer: Option<wgpu::Buffer>,
    // Widget bounds in physical pixels (for viewport in render pass)
    phys_bounds: [f32; 4],
}

// ---------------------------------------------------------------------------
// Pure math — testable without GPU or iced types
// ---------------------------------------------------------------------------

/// Compute the image rectangle in viewport-normalized UV [0,1].
pub fn compute_image_rect(
    image_w: f32,
    image_h: f32,
    viewport_w: f32,
    viewport_h: f32,
    zoom: f32,
    offset: [f32; 2],
) -> [f32; 4] {
    let fit = (viewport_w / image_w).min(viewport_h / image_h);
    let scale = fit * zoom;
    let dw = image_w * scale;
    let dh = image_h * scale;
    let left = (viewport_w - dw) / 2.0 + offset[0];
    let top = (viewport_h - dh) / 2.0 + offset[1];
    [
        left / viewport_w,
        top / viewport_h,
        (left + dw) / viewport_w,
        (top + dh) / viewport_h,
    ]
}

/// Compute new zoom and offset for a zoom-at-cursor operation.
/// Returns (new_zoom, new_offset).
pub fn zoom_at_cursor(
    zoom: f32,
    offset: [f32; 2],
    factor: f32,
    cursor: [f32; 2],
    canvas_size: [f32; 2],
) -> (f32, [f32; 2]) {
    let dx = cursor[0] - canvas_size[0] / 2.0;
    let dy = cursor[1] - canvas_size[1] / 2.0;
    let new_offset = [
        dx * (1.0 - factor) + offset[0] * factor,
        dy * (1.0 - factor) + offset[1] * factor,
    ];
    let new_zoom = (zoom * factor).clamp(0.01, 200.0);
    (new_zoom, new_offset)
}

// ---------------------------------------------------------------------------
// ImageCanvas -> iced Shader Program
// ---------------------------------------------------------------------------

impl ImageCanvas {
    fn compute_rect(&self, bounds: Rectangle) -> [f32; 4] {
        match &self.image {
            Some(img) => compute_image_rect(
                img.width as f32,
                img.height as f32,
                bounds.width,
                bounds.height,
                self.zoom,
                self.offset,
            ),
            None => [0.0; 4],
        }
    }
}

impl shader::Program<ViewerEvent> for ImageCanvas {
    type State = ViewerState;
    type Primitive = ImagePrimitive;

    fn update(
        &self,
        state: &mut ViewerState,
        event: shader::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        _shell: &mut iced::advanced::Shell<'_, ViewerEvent>,
    ) -> (event::Status, Option<ViewerEvent>) {
        match event {
            // ---- Zoom via scroll wheel ----
            shader::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.is_over(bounds) {
                    let y = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => y,
                        mouse::ScrollDelta::Pixels { y, .. } => y / 50.0,
                    };
                    let factor = 1.1_f32.powf(y);
                    let pos = cursor.position_in(bounds).unwrap_or_default();
                    return (
                        event::Status::Captured,
                        Some(ViewerEvent::Zoom {
                            factor,
                            cursor: [pos.x, pos.y],
                            canvas_size: [bounds.width, bounds.height],
                        }),
                    );
                }
            }

            // ---- Pan: drag start ----
            shader::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if cursor.is_over(bounds) {
                    state.dragging = true;
                    state.last_pos = cursor.position_in(bounds);
                    return (event::Status::Captured, None);
                }
            }

            // ---- Pan: drag end ----
            shader::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.dragging {
                    state.dragging = false;
                    state.last_pos = None;
                    return (event::Status::Captured, None);
                }
            }

            // ---- Pan: drag move ----
            shader::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.dragging {
                    if let Some(pos) = cursor.position_in(bounds) {
                        if let Some(last) = state.last_pos {
                            let dx = pos.x - last.x;
                            let dy = pos.y - last.y;
                            state.last_pos = Some(pos);
                            return (
                                event::Status::Captured,
                                Some(ViewerEvent::Pan { delta: [dx, dy] }),
                            );
                        }
                        state.last_pos = Some(pos);
                    }
                }
            }

            _ => {}
        }
        (event::Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &ViewerState,
        _cursor: mouse::Cursor,
        bounds: Rectangle,
    ) -> ImagePrimitive {
        ImagePrimitive {
            image: self.image.clone(),
            image_id: self.image_id,
            rect: self.compute_rect(bounds),
            adjustments: self.adjustments,
        }
    }

    fn mouse_interaction(
        &self,
        state: &ViewerState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if self.image.is_some() && cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

// ---------------------------------------------------------------------------
// ImagePrimitive -> iced shader::Primitive (GPU work)
// ---------------------------------------------------------------------------

impl shader::Primitive for ImagePrimitive {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        storage: &mut shader::Storage,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        let sf = viewport.scale_factor() as f32;

        // --- One-time GPU resource creation ---
        if !storage.has::<GpuResources>() {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("photo_shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../assets/shaders/image.wgsl").into(),
                ),
            });

            let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("photo_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("photo_pl"),
                bind_group_layouts: &[&bgl],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("photo_pipeline"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: "vs_main",
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("photo_sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("photo_uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            // 1x1 white placeholder for blur texture (until blur pass runs)
            let placeholder_tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("blur_placeholder"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &placeholder_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[255, 255, 255, 255],
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let placeholder_blur_view =
                placeholder_tex.create_view(&wgpu::TextureViewDescriptor::default());

            // Blur pipeline for clarity/dehaze pre-pass
            let blur_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("blur_shader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../assets/shaders/blur.wgsl").into(),
                ),
            });

            let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blur_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

            let blur_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("blur_pl"),
                bind_group_layouts: &[&blur_bgl],
                push_constant_ranges: &[],
            });

            let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("blur_pipeline"),
                layout: Some(&blur_pl),
                vertex: wgpu::VertexState {
                    module: &blur_module,
                    entry_point: "vs_main",
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &blur_module,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

            // Blur uniform buffer (vec2 direction + vec2 pad = 16 bytes)
            let blur_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("blur_uniforms"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            storage.store(GpuResources {
                pipeline,
                bind_group_layout: bgl,
                sampler,
                uniform_buffer,
                texture: None,
                texture_view: None,
                bind_group: None,
                current_image_id: 0,
                blur_texture_view: Some(placeholder_blur_view),
                blur_pipeline: Some(blur_pipeline),
                blur_bind_group_layout: Some(blur_bgl),
                blur_uniform_buffer: Some(blur_uniform_buffer),
                phys_bounds: [0.0; 4],
            });
        }

        let res = storage.get_mut::<GpuResources>().unwrap();

        // Physical-pixel bounds for the viewport in the render pass
        res.phys_bounds = [
            bounds.x * sf,
            bounds.y * sf,
            bounds.width * sf,
            bounds.height * sf,
        ];

        // --- Upload texture when image changes ---
        if let Some(img) = &self.image {
            if res.current_image_id != self.image_id || res.texture.is_none() {
                let max_dim = device.limits().max_texture_dimension_2d;
                let mut upload_w = img.width;
                let mut upload_h = img.height;
                let mut owned_pixels: Option<Vec<u8>> = None;

                // Downscale if image exceeds GPU texture limits
                if img.width > max_dim || img.height > max_dim {
                    let scale = max_dim as f32 / img.width.max(img.height) as f32;
                    let nw = ((img.width as f32 * scale) as u32).max(1);
                    let nh = ((img.height as f32 * scale) as u32).max(1);
                    if let Some(src) =
                        image::RgbaImage::from_raw(img.width, img.height, img.pixels.clone())
                    {
                        let resized = image::imageops::resize(
                            &src,
                            nw,
                            nh,
                            image::imageops::FilterType::Triangle,
                        );
                        upload_w = resized.width();
                        upload_h = resized.height();
                        owned_pixels = Some(resized.into_raw());
                    }
                }

                let pixels = owned_pixels.as_deref().unwrap_or(&img.pixels);

                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("photo_tex"),
                    size: wgpu::Extent3d {
                        width: upload_w,
                        height: upload_h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });

                queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture: &tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    pixels,
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * upload_w),
                        rows_per_image: Some(upload_h),
                    },
                    wgpu::Extent3d {
                        width: upload_w,
                        height: upload_h,
                        depth_or_array_layers: 1,
                    },
                );

                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

                // Generate blur texture for clarity/dehaze
                if let (Some(blur_pipeline), Some(blur_bgl), Some(blur_ubuf)) = (
                    &res.blur_pipeline,
                    &res.blur_bind_group_layout,
                    &res.blur_uniform_buffer,
                ) {
                    let blur_w = (upload_w / 4).max(1);
                    let blur_h = (upload_h / 4).max(1);

                    // Intermediate texture (horizontal blur output)
                    let intermediate_tex = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("blur_intermediate"),
                        size: wgpu::Extent3d {
                            width: blur_w,
                            height: blur_h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    });
                    let intermediate_view =
                        intermediate_tex.create_view(&wgpu::TextureViewDescriptor::default());

                    // Final blur texture
                    let blur_tex = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("blur_final"),
                        size: wgpu::Extent3d {
                            width: blur_w,
                            height: blur_h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    });
                    let blur_final_view =
                        blur_tex.create_view(&wgpu::TextureViewDescriptor::default());

                    // Horizontal blur pass: source = image texture -> intermediate
                    {
                        let h_dir: [f32; 4] = [1.0 / blur_w as f32, 0.0, 0.0, 0.0];
                        queue.write_buffer(blur_ubuf, 0, bytemuck::cast_slice(&h_dir));

                        let h_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("blur_h_bg"),
                            layout: blur_bgl,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: blur_ubuf.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&res.sampler),
                                },
                            ],
                        });

                        let mut encoder =
                            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("blur_h_enc"),
                            });
                        {
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("blur_h_pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &intermediate_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });
                            pass.set_pipeline(blur_pipeline);
                            pass.set_bind_group(0, &h_bg, &[]);
                            pass.draw(0..6, 0..1);
                        }
                        queue.submit(std::iter::once(encoder.finish()));
                    }

                    // Vertical blur pass: source = intermediate -> final blur
                    {
                        let v_dir: [f32; 4] = [0.0, 1.0 / blur_h as f32, 0.0, 0.0];
                        queue.write_buffer(blur_ubuf, 0, bytemuck::cast_slice(&v_dir));

                        let v_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("blur_v_bg"),
                            layout: blur_bgl,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: blur_ubuf.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(
                                        &intermediate_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&res.sampler),
                                },
                            ],
                        });

                        let mut encoder =
                            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("blur_v_enc"),
                            });
                        {
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("blur_v_pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &blur_final_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });
                            pass.set_pipeline(blur_pipeline);
                            pass.set_bind_group(0, &v_bg, &[]);
                            pass.draw(0..6, 0..1);
                        }
                        queue.submit(std::iter::once(encoder.finish()));
                    }

                    res.blur_texture_view = Some(blur_final_view);
                }

                // Create main bind group AFTER blur passes so it uses the
                // real blur texture view instead of the placeholder.
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("photo_bg"),
                    layout: &res.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: res.uniform_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&res.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(
                                res.blur_texture_view.as_ref().unwrap(),
                            ),
                        },
                    ],
                });

                res.texture = Some(tex);
                res.texture_view = Some(view);
                res.bind_group = Some(bg);
                res.current_image_id = self.image_id;
            }
        } else {
            res.texture = None;
            res.texture_view = None;
            res.bind_group = None;
            res.current_image_id = 0;
        }

        // --- Update uniform buffer every frame ---
        let adj = &self.adjustments;
        let identity_mat = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let mat = if adj.temp_matrix == [0.0; 9] {
            identity_mat
        } else {
            adj.temp_matrix
        };

        let uniforms = Uniforms {
            rect: self.rect,
            bg_color: [0.10, 0.10, 0.10, 1.0],
            exposure: adj.exposure,
            contrast: adj.contrast / 100.0,
            highlights: adj.highlights / 100.0,
            shadows: adj.shadows / 100.0,
            whites: adj.whites / 100.0,
            blacks: adj.blacks / 100.0,
            vibrance: adj.vibrance / 100.0,
            saturation: adj.saturation / 100.0,
            clarity: adj.clarity / 100.0,
            dehaze: adj.dehaze / 100.0,
            _pad0: 0.0,
            _pad1: 0.0,
            temp_mat_row0: [mat[0], mat[1], mat[2], 0.0],
            temp_mat_row1: [mat[3], mat[4], mat[5], 0.0],
            temp_mat_row2: [mat[6], mat[7], mat[8], 0.0],
            lens_enabled: if adj.lens_enabled { 1.0 } else { 0.0 },
            lens_dist_a: adj.lens_dist[0],
            lens_dist_b: adj.lens_dist[1],
            lens_dist_c: adj.lens_dist[2],
            lens_vig_k1: adj.lens_vig[0],
            lens_vig_k2: adj.lens_vig[1],
            lens_vig_k3: adj.lens_vig[2],
            lens_tca_r_scale: if adj.lens_tca_r == 0.0 {
                1.0
            } else {
                adj.lens_tca_r
            },
            lens_tca_b_scale: if adj.lens_tca_b == 0.0 {
                1.0
            } else {
                adj.lens_tca_b
            },
            image_aspect: adj.image_aspect,
            _pad2: 0.0,
            _pad3: 0.0,
        };
        queue.write_buffer(&res.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(res) = storage.get::<GpuResources>() else {
            return;
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("photo_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        let b = res.phys_bounds;
        pass.set_viewport(b[0], b[1], b[2].max(1.0), b[3].max(1.0), 0.0, 1.0);
        pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width.max(1),
            clip_bounds.height.max(1),
        );

        pass.set_pipeline(&res.pipeline);
        if let Some(bg) = &res.bind_group {
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3 // f32 precision across chained multiply/add
    }

    fn rect_approx_eq(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| approx_eq(*x, *y))
    }

    // -- compute_image_rect tests --

    #[test]
    fn fit_square_image_in_square_viewport() {
        // 100x100 image in 100x100 viewport, zoom=1 → fills exactly
        let r = compute_image_rect(100.0, 100.0, 100.0, 100.0, 1.0, [0.0, 0.0]);
        assert!(rect_approx_eq(r, [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn fit_wide_image_in_square_viewport() {
        // 200x100 image in 200x200 viewport → letterboxed (bars top/bottom)
        let r = compute_image_rect(200.0, 100.0, 200.0, 200.0, 1.0, [0.0, 0.0]);
        // fit = min(200/200, 200/100) = 1.0; dw=200, dh=100
        // left=0, top=50 → UV: [0, 0.25, 1.0, 0.75]
        assert!(rect_approx_eq(r, [0.0, 0.25, 1.0, 0.75]));
    }

    #[test]
    fn fit_tall_image_in_square_viewport() {
        // 100x200 image in 200x200 viewport → pillarboxed (bars left/right)
        let r = compute_image_rect(100.0, 200.0, 200.0, 200.0, 1.0, [0.0, 0.0]);
        // fit = min(200/100, 200/200) = 1.0; dw=100, dh=200
        // left=50, top=0 → UV: [0.25, 0, 0.75, 1.0]
        assert!(rect_approx_eq(r, [0.25, 0.0, 0.75, 1.0]));
    }

    #[test]
    fn zoom_2x_doubles_image_rect() {
        let r = compute_image_rect(100.0, 100.0, 100.0, 100.0, 2.0, [0.0, 0.0]);
        // scale = 1.0 * 2.0 = 2.0; dw=200, dh=200; centered at (50,50)
        // left = (100-200)/2 = -50; top = -50
        // UV: [-0.5, -0.5, 1.5, 1.5]
        assert!(rect_approx_eq(r, [-0.5, -0.5, 1.5, 1.5]));
    }

    #[test]
    fn pan_offset_shifts_rect() {
        // 100x100 in 100x100, zoom=1, pan right 20px
        let r = compute_image_rect(100.0, 100.0, 100.0, 100.0, 1.0, [20.0, 0.0]);
        // left = 0 + 20 = 20, UV: [0.2, 0, 1.2, 1.0]
        assert!(rect_approx_eq(r, [0.2, 0.0, 1.2, 1.0]));
    }

    #[test]
    fn image_rect_centered_for_different_aspect_ratios() {
        // 1920x1080 image in 800x600 viewport
        let r = compute_image_rect(1920.0, 1080.0, 800.0, 600.0, 1.0, [0.0, 0.0]);
        // fit = min(800/1920, 600/1080) = min(0.4167, 0.5556) = 0.4167
        // dw = 1920 * 0.4167 = 800, dh = 1080 * 0.4167 = 450
        // left = 0, top = (600-450)/2 = 75
        // UV: [0, 75/600, 1.0, 525/600] = [0, 0.125, 1.0, 0.875]
        assert!(approx_eq(r[0], 0.0));
        assert!(approx_eq(r[2], 1.0)); // fills width
        assert!(r[1] > 0.0); // top margin
        assert!(r[3] < 1.0); // bottom margin
        assert!(approx_eq(r[1], 1.0 - r[3])); // symmetric
    }

    // -- zoom_at_cursor tests --

    #[test]
    fn zoom_at_center_does_not_change_offset() {
        // Zooming at the exact center should not shift the offset
        let (z, o) = zoom_at_cursor(1.0, [0.0, 0.0], 2.0, [400.0, 300.0], [800.0, 600.0]);
        assert!(approx_eq(z, 2.0));
        assert!(approx_eq(o[0], 0.0));
        assert!(approx_eq(o[1], 0.0));
    }

    #[test]
    fn zoom_at_corner_shifts_offset() {
        // Zoom 2x at top-left corner (0,0) of an 800x600 canvas
        let (z, o) = zoom_at_cursor(1.0, [0.0, 0.0], 2.0, [0.0, 0.0], [800.0, 600.0]);
        assert!(approx_eq(z, 2.0));
        // dx = 0 - 400 = -400; new_offset_x = -400*(1-2) + 0*2 = 400
        assert!(approx_eq(o[0], 400.0));
        assert!(approx_eq(o[1], 300.0));
    }

    #[test]
    fn zoom_preserves_point_under_cursor() {
        // The image point under the cursor should map to the same cursor position
        // before and after zoom.
        let canvas = [800.0, 600.0];
        let cursor = [200.0, 150.0];
        let image = (1920.0_f32, 1080.0_f32);
        let zoom = 1.5_f32;
        let offset = [10.0_f32, -20.0];
        let factor = 1.3_f32;

        // Compute image point under cursor before zoom
        let fit = (canvas[0] / image.0).min(canvas[1] / image.1);
        let scale_before = fit * zoom;
        let img_x = (cursor[0] - canvas[0] / 2.0 - offset[0]) / scale_before;
        let img_y = (cursor[1] - canvas[1] / 2.0 - offset[1]) / scale_before;

        let (new_zoom, new_offset) = zoom_at_cursor(zoom, offset, factor, cursor, canvas);

        // Same image point after zoom
        let scale_after = fit * new_zoom;
        let screen_x = img_x * scale_after + canvas[0] / 2.0 + new_offset[0];
        let screen_y = img_y * scale_after + canvas[1] / 2.0 + new_offset[1];

        assert!(approx_eq(screen_x, cursor[0]));
        assert!(approx_eq(screen_y, cursor[1]));
    }

    #[test]
    fn zoom_clamps_to_limits() {
        let (z, _) = zoom_at_cursor(0.02, [0.0, 0.0], 0.1, [0.0, 0.0], [800.0, 600.0]);
        assert!(approx_eq(z, 0.01)); // min clamp

        let (z, _) = zoom_at_cursor(150.0, [0.0, 0.0], 2.0, [0.0, 0.0], [800.0, 600.0]);
        assert!(approx_eq(z, 200.0)); // max clamp
    }
}
