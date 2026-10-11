//! GPU renderer. One shared `Gpu` (device, pipelines, glyph atlas) serves every window; each
//! window only owns a `Surface` (swapchain + a few small buffers). Per frame: one instanced draw
//! for cell backgrounds, one for free-form quads (cursor, trail, underlines), one for glyphs.
//! Nothing is allocated per frame.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use crossfont::{BitmapBuffer, FontDesc, FontKey, GlyphKey, Rasterize, Rasterizer, Size, Slant, Style, Weight};
use winit::window::Window;

/// Tried after the configured family.
const FALLBACK_FONTS: [&str; 4] = ["JetBrainsMono Nerd Font", "JetBrains Mono", "Cascadia Mono", "Consolas"];
const ATLAS: u32 = 2048;
const MAX_GLYPHS: usize = 8192;
const MAX_CELLS: usize = 1 << 16;
const MAX_QUADS: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CellInst {
    pub pos: u32, // col | row << 16
    pub glyph: u32,
    pub fg: u32, // rgba8, r in the low byte
    pub bg: u32, // alpha 0 = default background
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct QuadInst {
    pub pts: [[f32; 2]; 4], // tl, tr, br, bl
    pub color: u32,
}

pub fn rect(x: f32, y: f32, w: f32, h: f32, color: u32) -> QuadInst {
    QuadInst { pts: [[x, y], [x + w, y], [x + w, y + h], [x, y + h]], color }
}

pub fn pack([r, g, b]: [u8; 3], a: u8) -> u32 {
    u32::from_le_bytes([r, g, b, a])
}

/// One font family at one pixel size (windows on monitors with different DPI need different sizes).
struct Face {
    fonts: [FontKey; 4], // regular, bold, italic, bold-italic
    size: Size,
    cell: (f32, f32, f32), // width, height, baseline
}

/// Font rasterization and atlas packing, shared by all windows.
struct Glyphs {
    raster: Rasterizer,
    family: String,
    pt: f32,
    faces: HashMap<u32, Face>, // by scale * 100
    cache: HashMap<(char, u8, u32), u32>,
    table: Vec<[f32; 8]>, // per glyph: atlas x, y, w, h, left, top, -, -
    shelf: (u32, u32, u32), // x, y, row height
}

fn scale_key(scale: f32) -> u32 {
    (scale * 100.0).round() as u32
}

impl Glyphs {
    fn face(&mut self, scale: f32) -> &Face {
        let (raster, family, pt) = (&mut self.raster, &self.family, self.pt);
        self.faces.entry(scale_key(scale)).or_insert_with(|| {
            let size = Size::new(pt * scale);
            let mut load = |weight, slant| {
                let style = Style::Description { weight, slant };
                std::iter::once(family.as_str())
                    .chain(FALLBACK_FONTS)
                    .find_map(|name| raster.load_font(&FontDesc::new(name, style.clone()), size).ok())
            };
            let regular = load(Weight::Normal, Slant::Normal).expect("no monospace font found");
            let fonts = [
                regular,
                load(Weight::Bold, Slant::Normal).unwrap_or(regular),
                load(Weight::Normal, Slant::Italic).unwrap_or(regular),
                load(Weight::Bold, Slant::Italic).unwrap_or(regular),
            ];
            let m = raster.metrics(regular, size).expect("font metrics");
            let (w, h) = (m.average_advance.round() as f32, m.line_height.round() as f32);
            Face { fonts, size, cell: (w, h, (h + m.descent).round()) }
        })
    }

    fn clear(&mut self) {
        self.cache.clear();
        self.table.truncate(1);
        self.shelf = (0, 0, 0);
    }
}

pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    bg_pipe: wgpu::RenderPipeline,
    fg_pipe: wgpu::RenderPipeline,
    quad_pipe: wgpu::RenderPipeline,
    glyph_buf: wgpu::Buffer,
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    glyphs: Glyphs,
    /// Set when the atlas was reset mid-frame; windows should redraw.
    pub atlas_reset: bool,
}

/// A window's swapchain and per-window buffers.
pub struct Surface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    uniforms: wgpu::Buffer,
    cell_buf: wgpu::Buffer,
    quad_buf: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

impl Gpu {
    /// Set up the GPU against the first window, returning its surface too.
    pub fn new(window: Arc<Window>, font_family: &str, font_pt: f32) -> (Self, Surface) {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::DX12;
        // A DirectComposition swapchain is what allows a translucent window.
        desc.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        let instance = wgpu::Instance::new(desc);
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            // A terminal needs a few MB of GPU memory; the default hint reserves large heaps.
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .expect("device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);

        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let glyph_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyphs"),
            size: (MAX_GLYPHS * 32) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                entry(1, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                entry(2, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                }),
            ],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let cell_attrs = wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32, 2 => Uint32, 3 => Uint32];
        let quad_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Uint32];
        let pipeline = |vs: &str, fs: &str, stride: usize, attrs: &[wgpu::VertexAttribute]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&pipe_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: stride as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: attrs,
                    })],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let bg_pipe = pipeline("vs_bg", "fs_bg", size_of::<CellInst>(), &cell_attrs);
        let fg_pipe = pipeline("vs_fg", "fs_fg", size_of::<CellInst>(), &cell_attrs);
        let quad_pipe = pipeline("vs_quad", "fs_bg", size_of::<QuadInst>(), &quad_attrs);

        let glyphs = Glyphs {
            raster: Rasterizer::new().expect("DirectWrite unavailable"),
            family: font_family.into(),
            pt: font_pt,
            faces: HashMap::new(),
            cache: HashMap::new(),
            table: vec![[0.0; 8]], // index 0 = no glyph
            shelf: (0, 0, 0),
        };
        let mut gpu = Self {
            instance,
            adapter,
            device,
            queue,
            format,
            layout,
            bg_pipe,
            fg_pipe,
            quad_pipe,
            glyph_buf,
            atlas,
            atlas_view,
            glyphs,
            atlas_reset: false,
        };
        let surface = gpu.wrap_surface(surface, window.inner_size());
        (gpu, surface)
    }

    pub fn surface(&mut self, window: Arc<Window>) -> Surface {
        let size = window.inner_size();
        let surface = self.instance.create_surface(window).expect("surface");
        self.wrap_surface(surface, size)
    }

    fn wrap_surface(&mut self, surface: wgpu::Surface<'static>, size: winit::dpi::PhysicalSize<u32>) -> Surface {
        let caps = surface.get_capabilities(&self.adapter);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            caps.alpha_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&self.device, &config);
        let buffer = |label, size: usize, usage| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let uniforms = buffer("uniforms", 48, wgpu::BufferUsages::UNIFORM);
        let cell_buf = buffer("cells", MAX_CELLS * size_of::<CellInst>(), wgpu::BufferUsages::VERTEX);
        let quad_buf = buffer("quads", MAX_QUADS * size_of::<QuadInst>(), wgpu::BufferUsages::VERTEX);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.glyph_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
            ],
        });
        Surface { surface, config, uniforms, cell_buf, quad_buf, bind }
    }

    pub fn resize(&self, s: &mut Surface, w: u32, h: u32) {
        s.config.width = w.max(1);
        s.config.height = h.max(1);
        s.surface.configure(&self.device, &s.config);
    }

    /// Cell width, height and baseline in physical pixels at a window scale factor.
    pub fn cell(&mut self, scale: f32) -> (f32, f32, f32) {
        self.glyphs.face(scale).cell
    }

    /// Switch font; returns true if anything changed (windows must then re-layout).
    pub fn set_font(&mut self, family: &str, pt: f32) -> bool {
        if self.glyphs.family == family && self.glyphs.pt == pt {
            return false;
        }
        self.glyphs.family = family.into();
        self.glyphs.pt = pt;
        self.glyphs.faces.clear();
        self.glyphs.clear();
        true
    }

    /// Atlas index of a glyph, rasterizing it on first use. 0 means nothing to draw.
    pub fn glyph(&mut self, c: char, style: u8, scale: f32) -> u32 {
        if c == ' ' {
            return 0;
        }
        let sk = scale_key(scale);
        if let Some(&i) = self.glyphs.cache.get(&(c, style, sk)) {
            return i;
        }
        let face = self.glyphs.face(scale);
        let key = GlyphKey { character: c, font_key: face.fonts[style as usize], size: face.size };
        let g = &mut self.glyphs;
        let r = match g.raster.get_glyph(key) {
            Ok(r) | Err(crossfont::Error::MissingGlyph(r)) => r,
            Err(_) => return 0,
        };
        let (w, h) = (r.width.max(0) as u32, r.height.max(0) as u32);
        if w == 0 || h == 0 {
            g.cache.insert((c, style, sk), 0);
            return 0;
        }
        // Shelf packing; when the atlas or table fills up, start over.
        // ponytail: full reset on overflow (one glitched frame), LRU eviction if it ever matters.
        if g.shelf.0 + w > ATLAS {
            g.shelf = (0, g.shelf.1 + g.shelf.2 + 1, 0);
        }
        if g.shelf.1 + h > ATLAS || g.table.len() >= MAX_GLYPHS {
            g.clear();
            self.atlas_reset = true;
        }
        let (x, y) = (g.shelf.0, g.shelf.1);
        g.shelf.0 += w + 1;
        g.shelf.2 = g.shelf.2.max(h);

        let pixels: Vec<u8> = match &r.buffer {
            // ClearType 3x1 coverage -> grayscale, like kitty on Linux; correct on translucent bg.
            BitmapBuffer::Rgb(v) => v.chunks_exact(3).map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8).collect(),
            BitmapBuffer::Rgba(v) => v.chunks_exact(4).map(|p| p[3]).collect(),
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let idx = g.table.len() as u32;
        let entry = [x as f32, y as f32, w as f32, h as f32, r.left as f32, r.top as f32, 0.0, 0.0];
        g.table.push(entry);
        self.queue.write_buffer(&self.glyph_buf, idx as u64 * 32, bytemuck::cast_slice(&entry));
        g.cache.insert((c, style, sk), idx);
        idx
    }

    /// `clear` is premultiplied RGBA. `zoom`/`opacity` drive the open/close animation.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        s: &mut Surface,
        scale: f32,
        cells: &[CellInst],
        quads: &[QuadInst],
        clear: [f64; 4],
        pad: f32,
        zoom: f32,
        opacity: f32,
    ) {
        let frame = match s.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                s.surface.configure(&self.device, &s.config);
                return;
            }
            _ => return,
        };
        let cells = &cells[..cells.len().min(MAX_CELLS)];
        let quads = &quads[..quads.len().min(MAX_QUADS)];
        let (cw, ch, base) = self.cell(scale);
        let (sw, sh) = (s.config.width as f32, s.config.height as f32);
        let u = [sw, sh, cw, ch, pad, pad, sw / 2.0, sh / 2.0, zoom, opacity, base, 0.0];
        self.queue.write_buffer(&s.uniforms, 0, bytemuck::cast_slice(&u));
        self.queue.write_buffer(&s.cell_buf, 0, bytemuck::cast_slice(cells));
        self.queue.write_buffer(&s.quad_buf, 0, bytemuck::cast_slice(quads));

        let view = frame.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0], g: clear[1], b: clear[2], a: clear[3] }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &s.bind, &[]);
            let n = cells.len() as u32;
            pass.set_vertex_buffer(0, s.cell_buf.slice(..));
            pass.set_pipeline(&self.bg_pipe);
            pass.draw(0..6, 0..n);
            if !quads.is_empty() {
                pass.set_pipeline(&self.quad_pipe);
                pass.set_vertex_buffer(0, s.quad_buf.slice(..));
                pass.draw(0..6, 0..quads.len() as u32);
            }
            pass.set_pipeline(&self.fg_pipe);
            pass.set_vertex_buffer(0, s.cell_buf.slice(..));
            pass.draw(0..6, 0..n);
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
    }
}
