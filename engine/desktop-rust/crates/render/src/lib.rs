//! 描画の指示 (`core::draw::DrawList`) を GPU で描く (wgpu)。
//!
//! 外から見えるのは [`Renderer`] と [`RenderError`] だけで、wgpu / etagere の型は出さない。
//! 描く先は raw-window-handle のトレイトを実装したウィンドウなら何でもよい
//! (ウィンドウを作るのは `biwa-player-platform`)。
//!
//! 描き方は 1 通りしかない。塗りの矩形もグリフも「インスタンスの矩形」として 1 本のパイプラインに流し、
//! 1 回の描画呼び出しで描く。重なりの順は描画の指示の順のまま保たれる。
//! グリフはアルファだけのアトラス (`atlas.rs`) に載せ、色はシェーダで掛ける。
//!
//! - バックエンドは wgpu が選ぶ (Vulkan / Metal / DX12 / GL)。
//! - ノベルゲームに独立 GPU は要らないので、省電力の GPU を選ぶ。
//! - vsync (`Fifo`) で描く。描き直しが要らないときに呼ばないのは呼ぶ側 (player) の仕事。

mod atlas;

use std::fmt;
use std::sync::Arc;

use biwa_player_core::draw::{DrawItem, DrawList};
use biwa_player_core::geometry::Color;
use biwa_player_core::text::GlyphRasterizer;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use atlas::GlyphAtlas;

/// 描画のエラー。
#[derive(Debug, Clone)]
pub struct RenderError(String);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

fn error(context: &str, e: impl fmt::Display) -> RenderError {
    RenderError(format!("{context}: {e}"))
}

/// 描く先のウィンドウ。
pub trait RenderTarget: HasWindowHandle + HasDisplayHandle + fmt::Debug + Send + Sync + 'static {}

impl<T: HasWindowHandle + HasDisplayHandle + fmt::Debug + Send + Sync + 'static> RenderTarget for T {}

/// インスタンス 1 つ (= 矩形 1 つ)。シェーダの `Instance` と同じ並び。
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    mode: u32,
    _pad: [u32; 3],
}

const MODE_FILL: u32 = 0;
const MODE_GLYPH: u32 = 1;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    atlas: GlyphAtlas,
    adapter: String,
}

impl Renderer {
    /// `target` に描く準備をする。`width` / `height` は物理ピクセル。
    pub fn new<T: RenderTarget>(target: Arc<T>, width: u32, height: u32) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
            Box::new(target.clone()),
        ));
        let surface = instance.create_surface(target).map_err(|e| error("failed to create a surface", e))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| error("no GPU adapter is available", e))?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?})", info.name, info.backend);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("biwa"),
            required_features: wgpu::Features::empty(),
            // GL (古い GPU の逃げ道) でも動く範囲に留める。
            required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| error("failed to open the GPU device", e))?;

        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| RenderError("the surface is not supported by the GPU adapter".into()))?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(wgpu::TextureFormat::is_srgb) {
            config.format = format;
        }
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("biwa shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("biwa globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("biwa pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("biwa pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("biwa globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas = GlyphAtlas::new(&device);
        let atlas_view = atlas.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("biwa atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("biwa globals"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let instance_capacity = 1024;
        let instances = Self::instance_buffer(&device, instance_capacity);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            globals,
            bind_group,
            instances,
            instance_capacity,
            atlas,
            adapter: adapter_name,
        })
    }

    /// 選ばれた GPU (ログ用)。
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// 描く先の大きさが変わった (物理ピクセル)。
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// 1 フレーム描く。グリフの画像が要れば `glyphs` に頼む。
    pub fn render(&mut self, list: &DrawList, glyphs: &mut dyn GlyphRasterizer) -> Result<(), RenderError> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // ウィンドウが隠れている・大きさが変わった途中など。次のフレームでやり直す。
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            other => return Err(RenderError(format!("failed to acquire a frame: {other:?}"))),
        };

        let instances = self.build_instances(list, glyphs);
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = Self::instance_buffer(&self.device, self.instance_capacity);
        }
        if !instances.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        }
        let size = [self.config.width as f32, self.config.height as f32, 0.0, 0.0];
        self.queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&size));

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("biwa frame") });
        {
            let clear = linear(list.clear);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("biwa frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(clear[0]),
                            g: f64::from(clear[1]),
                            b: f64::from(clear[2]),
                            a: f64::from(clear[3]),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !instances.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                pass.draw(0..6, 0..instances.len() as u32);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }

    fn build_instances(&mut self, list: &DrawList, glyphs: &mut dyn GlyphRasterizer) -> Vec<Instance> {
        let mut out = Vec::with_capacity(list.items.len());
        for item in &list.items {
            match *item {
                DrawItem::Rect { rect, color } => out.push(Instance {
                    rect: [rect.x, rect.y, rect.width, rect.height],
                    uv: [0.0; 4],
                    color: linear(color),
                    mode: MODE_FILL,
                    _pad: [0; 3],
                }),
                DrawItem::Glyph { key, x, y, color } => {
                    let Some(g) = self.atlas.get(key, glyphs, &self.queue) else {
                        continue;
                    };
                    // ピクセルの格子に合わせてにじまないようにする。
                    let left = (x + g.left).round();
                    let top = (y + g.top).round();
                    out.push(Instance {
                        rect: [left, top, g.width, g.height],
                        uv: g.uv,
                        color: linear(color),
                        mode: MODE_GLYPH,
                        _pad: [0; 3],
                    });
                }
            }
        }
        out
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("biwa instances"),
            size: (capacity * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
}

/// sRGB の 8 bit の色を、線形の 0〜1 にする (描画先は sRGB の形式なので GPU が戻す)。
fn linear(c: Color) -> [f32; 4] {
    let ch = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    [ch(c.r), ch(c.g), ch(c.b), f32::from(c.a) / 255.0]
}
