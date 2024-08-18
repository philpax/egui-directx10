#![warn(missing_docs)]

//! `egui-directx10`: a Direct3D10 renderer for [`egui`](https://crates.io/crates/egui).
//!
//! This crate aims to provide a *minimal* set of features and APIs to render
//! outputs from `egui` using Direct3D10. We assume you to be familiar with
//! developing graphics applications using Direct3D10, and if not, this crate is
//! not likely useful for you. Besides, this crate cares only about rendering
//! outputs from `egui`, so it is all *your* responsibility to handle things
//! like setting up the window and event loop, creating the device and swap
//! chain, etc.
//!
//! This crate is built upon the *official* Rust bindings of Direct3D10 and DXGI
//! APIs from the [`windows`](https://crates.io/crates/windows) crate [maintained by
//! Microsoft](https://github.com/microsoft/windows-rs). Using this crate with
//! other Direct3D10 bindings is not recommended and may result in unexpected
//! behavior.
//!
//! This crate is in early development. It should work in most cases but may
//! lack certain features or functionalities.
//!
//! To get started, you can check the [`Renderer`] struct provided by this
//! crate. You can also take a look at the [`egui-demo`](https://github.com/Nekomaru-PKU/egui-directx10/blob/main/examples/egui-demo.rs) example, which demonstrates all you need to do to set up a minimal application
//! with Direct3D10 and `egui`. This example uses `winit` for window management
//! and event handling, while native Win32 APIs should also work well.

mod texture;
use texture::TexturePool;

use std::mem;

const fn zeroed<T>() -> T {
    unsafe { mem::zeroed() }
}

use egui::{
    epaint::{textures::TexturesDelta, ClippedShape, Primitive, Vertex},
    ClippedPrimitive, Pos2, Rgba,
};

use windows::{
    core::{Interface, Result},
    Win32::{
        Foundation::{BOOL, RECT},
        Graphics::{Direct3D::*, Direct3D10::*, Dxgi::Common::*},
    },
};

/// The core of this crate. You can set up a renderer via [`Renderer::new`]
/// and render the output from `egui` with [`Renderer::render`].
pub struct Renderer {
    device: ID3D10Device,

    input_layout: ID3D10InputLayout,
    vertex_shader: ID3D10VertexShader,
    pixel_shader: ID3D10PixelShader,
    rasterizer_state: ID3D10RasterizerState,
    depth_stencil_state: ID3D10DepthStencilState,
    sampler_state: ID3D10SamplerState,
    blend_state: ID3D10BlendState,

    texture_pool: TexturePool,

    restore_state_after_render: bool,
    state_block: Option<StateBlock>,
}

/// Part of [`egui::FullOutput`] that is consumed by [`Renderer::render`].
///
/// Call to [`egui::Context::run`] or [`egui::Context::end_frame`] yields a
/// [`egui::FullOutput`]. The platform integration (for example `egui_winit`)
/// consumes [`egui::FullOutput::platform_output`] and
/// [`egui::FullOutput::viewport_output`], and the renderer consumes the rest.
///
/// To conveniently split a [`egui::FullOutput`] into a [`RendererOutput`] and
/// outputs for the platform integration, use [`split_output`].
#[allow(missing_docs)]
pub struct RendererOutput {
    pub textures_delta: TexturesDelta,
    pub shapes: Vec<ClippedShape>,
    pub pixels_per_point: f32,
}

/// Convenience method to split a [`egui::FullOutput`] into the
/// [`RendererOutput`] part and other parts for platform integration.
pub fn split_output(
    full_output: egui::FullOutput,
) -> (
    RendererOutput,
    egui::PlatformOutput,
    egui::ViewportIdMap<egui::ViewportOutput>,
) {
    (
        RendererOutput {
            textures_delta: full_output.textures_delta,
            shapes: full_output.shapes,
            pixels_per_point: full_output.pixels_per_point,
        },
        full_output.platform_output,
        full_output.viewport_output,
    )
}

#[repr(C)]
struct VertexData {
    pos: Pos2,
    uv: Pos2,
    color: Rgba,
}

struct MeshData {
    vtx: Vec<VertexData>,
    idx: Vec<u32>,
    tex: egui::TextureId,
    clip_rect: egui::Rect,
}

impl Renderer {
    /// Create a [`Renderer`] using the provided Direct3D10 device. The
    /// [`Renderer`] holds various Direct3D10 resources and states derived
    /// from the device.
    ///
    /// If any Direct3D resource creation fails, this function will return an
    /// error. You can create the Direct3D10 device with debug layer enabled
    /// to find out details on the error.
    pub fn new(
        device: &ID3D10Device,
        gamma_output: bool,
        restore_state_after_render: bool,
    ) -> Result<Self> {
        let mut input_layout = None;
        let mut vertex_shader = None;
        let mut pixel_shader = None;
        let mut rasterizer_state = None;
        let mut depth_stencil_state = None;
        let mut sampler_state = None;
        let mut blend_state = None;
        unsafe {
            device.CreateInputLayout(
                &Self::INPUT_ELEMENTS_DESC,
                Self::VS_BLOB,
                Some(&mut input_layout),
            )?;
            device
                .CreateVertexShader(Self::VS_BLOB, Some(&mut vertex_shader))?;
            device.CreatePixelShader(
                if gamma_output {
                    Self::PS_GAMMA_BLOB
                } else {
                    Self::PS_LINEAR_BLOB
                },
                Some(&mut pixel_shader),
            )?;
            device.CreateRasterizerState(
                &Self::RASTERIZER_DESC,
                Some(&mut rasterizer_state),
            )?;
            device.CreateDepthStencilState(
                &Self::DEPTH_STENCIL_DESC,
                Some(&mut depth_stencil_state),
            )?;
            device.CreateSamplerState(
                &Self::SAMPLER_DESC,
                Some(&mut sampler_state),
            )?;
            device
                .CreateBlendState(&Self::BLEND_DESC, Some(&mut blend_state))?;
        };
        Ok(Self {
            device: device.clone(),
            input_layout: input_layout.unwrap(),
            vertex_shader: vertex_shader.unwrap(),
            pixel_shader: pixel_shader.unwrap(),
            rasterizer_state: rasterizer_state.unwrap(),
            depth_stencil_state: depth_stencil_state.unwrap(),
            sampler_state: sampler_state.unwrap(),
            blend_state: blend_state.unwrap(),
            texture_pool: TexturePool::new(device),
            restore_state_after_render,
            state_block: None,
        })
    }

    /// Render the output of `egui` to the provided render target using the
    /// provided device context. The render target should use a linear color
    /// space (e.g. `DXGI_FORMAT_R8G8B8A8_UNORM_SRGB`) for proper results.
    ///
    /// The `scale_factor` should be the scale factor of your window and not
    /// confused with [`egui::Context::zoom_factor`]. If you are using `winit`,
    /// the `scale_factor` can be aquired using `Window::scale_factor`.
    ///
    /// ## Error Handling
    ///
    /// If any Direct3D resource creation fails, this function will return an
    /// error. In this case you may have a incomplete or incorrect rendering
    /// result. You can create the Direct3D10 device with debug layer
    /// enabled to find out details on the error.
    /// If the device has been lost, you should drop the [`Renderer`] and create
    /// a new one.
    ///
    /// ## Pipeline State Management
    ///
    /// This function sets up its own Direct3D10 pipeline state for rendering on
    /// the provided device context. It assumes that the hull shader, domain
    /// shader and geometry shader stages are not active on the provided device
    /// context without any further checks. It is all *your* responsibility to
    /// backup the current pipeline state and restore it afterwards if your
    /// rendering pipeline depends on it.
    ///
    /// Particularly, it overrides:
    /// + The input layout, vertex buffer, index buffer and primitive topology
    ///   in the input assembly stage;
    /// + The current shader in the vertex shader stage;
    /// + The viewport and rasterizer state in the rasterizer stage;
    /// + The current shader, shader resource slot 0 and sampler slot 0 in the
    ///   pixel shader stage;
    /// + The render target(s) and blend state in the output merger stage;
    ///
    /// See the [`egui-demo`](https://github.com/Nekomaru-PKU/egui-directx10/blob/main/examples/egui-demo.rs)
    /// example for code examples.
    pub fn render(
        &mut self,
        device_context: &ID3D10Device,
        render_target: &ID3D10RenderTargetView,
        depth_stencil_target: &ID3D10DepthStencilView,
        egui_ctx: &egui::Context,
        egui_output: RendererOutput,
        scale_factor: f32,
    ) -> Result<()> {
        self.texture_pool
            .update(device_context, egui_output.textures_delta)?;

        if egui_output.shapes.is_empty() {
            return Ok(());
        }

        let frame_size = Self::get_render_target_size(render_target)?;
        let frame_size_scaled = (
            frame_size.0 as f32 / scale_factor,
            frame_size.1 as f32 / scale_factor,
        );
        let zoom_factor = egui_ctx.zoom_factor();

        if self.restore_state_after_render {
            self.state_block =
                Some(unsafe { StateBlock::new(device_context)? });
        }

        self.setup(
            device_context,
            render_target,
            depth_stencil_target,
            frame_size,
        );
        let meshes = egui_ctx
            .tessellate(egui_output.shapes, egui_output.pixels_per_point)
            .into_iter()
            .filter_map(
                |ClippedPrimitive {
                     primitive,
                     clip_rect,
                 }| match primitive {
                    Primitive::Mesh(mesh) => Some((mesh, clip_rect)),
                    Primitive::Callback(..) => {
                        log::warn!("paint callbacks are not yet supported.");
                        None
                    },
                },
            )
            .filter_map(|(mesh, clip_rect)| {
                if mesh.indices.is_empty() {
                    return None;
                }
                if mesh.indices.len() % 3 != 0 {
                    log::warn!(concat!(
                        "egui wants to draw a incomplete triangle. ",
                        "this request will be ignored."
                    ));
                    return None;
                }
                Some(MeshData {
                    vtx: mesh
                        .vertices
                        .into_iter()
                        .map(|Vertex { pos, uv, color }| VertexData {
                            pos: Pos2::new(
                                pos.x * zoom_factor / frame_size_scaled.0 * 2.0
                                    - 1.0,
                                1.0 - pos.y * zoom_factor / frame_size_scaled.1
                                    * 2.0,
                            ),
                            uv,
                            color: color.into(),
                        })
                        .collect(),
                    idx: mesh.indices,
                    tex: mesh.texture_id,
                    clip_rect: clip_rect * scale_factor * zoom_factor,
                })
            });
        for mesh in meshes {
            Self::draw_mesh(
                &self.device,
                device_context,
                &self.texture_pool,
                mesh,
            )?;
        }

        if let Some(state_block) = self.state_block.as_ref() {
            unsafe { state_block.apply(device_context) };
        }

        Ok(())
    }

    fn setup(
        &mut self,
        ctx: &ID3D10Device,
        render_target: &ID3D10RenderTargetView,
        depth_stencil_target: &ID3D10DepthStencilView,
        frame_size: (u32, u32),
    ) {
        unsafe {
            ctx.IASetPrimitiveTopology(D3D10_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(&self.input_layout);
            ctx.VSSetShader(&self.vertex_shader);
            ctx.PSSetShader(&self.pixel_shader);
            ctx.RSSetState(&self.rasterizer_state);
            ctx.RSSetViewports(Some(&[D3D10_VIEWPORT {
                TopLeftX: 0,
                TopLeftY: 0,
                Width: frame_size.0 as _,
                Height: frame_size.1 as _,
                MinDepth: 0.,
                MaxDepth: 1.,
            }]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler_state.clone())]));
            ctx.OMSetRenderTargets(
                Some(&[Some(render_target.clone())]),
                Some(depth_stencil_target),
            );
            ctx.OMSetBlendState(&self.blend_state, &[0.; 4], u32::MAX);
            ctx.OMSetDepthStencilState(&self.depth_stencil_state, 2);
        }
    }

    fn draw_mesh(
        device: &ID3D10Device,
        device_context: &ID3D10Device,
        texture_pool: &TexturePool,
        mesh: MeshData,
    ) -> Result<()> {
        let ib = Self::create_index_buffer(device, &mesh.idx)?;
        let vb = Self::create_vertex_buffer(device, &mesh.vtx)?;
        unsafe {
            device_context.IASetVertexBuffers(
                0,
                1,
                Some(&Some(vb.clone())),
                Some(&(mem::size_of::<VertexData>() as _)),
                Some(&0),
            );
            device_context.IASetIndexBuffer(&ib, DXGI_FORMAT_R32_UINT, 0);
            device_context.RSSetScissorRects(Some(&[RECT {
                left: mesh.clip_rect.left() as _,
                top: mesh.clip_rect.top() as _,
                right: mesh.clip_rect.right() as _,
                bottom: mesh.clip_rect.bottom() as _,
            }]));
        }
        if let Some(srv) = texture_pool.get_srv(mesh.tex) {
            unsafe {
                device_context.PSSetShaderResources(0, Some(&[Some(srv)]))
            };
        } else {
            log::warn!(
                concat!(
                    "egui wants to sample a non-existing texture {:?}.",
                    "this request will be ignored."
                ),
                mesh.tex
            );
        };
        unsafe { device_context.DrawIndexed(mesh.idx.len() as _, 0, 0) };
        Ok(())
    }
}

impl Renderer {
    const VS_BLOB: &'static [u8] = include_bytes!("../shaders/egui_vs.bin");
    const PS_LINEAR_BLOB: &'static [u8] =
        include_bytes!("../shaders/egui_ps_linear.bin");
    const PS_GAMMA_BLOB: &'static [u8] =
        include_bytes!("../shaders/egui_ps_gamma.bin");

    const INPUT_ELEMENTS_DESC: [D3D10_INPUT_ELEMENT_DESC; 3] = [
        D3D10_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("POSITION"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D10_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D10_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("TEXCOORD"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: D3D10_APPEND_ALIGNED_ELEMENT,
            InputSlotClass: D3D10_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D10_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::s!("COLOR"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: D3D10_APPEND_ALIGNED_ELEMENT,
            InputSlotClass: D3D10_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    const RASTERIZER_DESC: D3D10_RASTERIZER_DESC = D3D10_RASTERIZER_DESC {
        FillMode: D3D10_FILL_SOLID,
        CullMode: D3D10_CULL_NONE,
        FrontCounterClockwise: BOOL(1),
        DepthBias: 0,
        DepthBiasClamp: 0.,
        SlopeScaledDepthBias: 0.,
        DepthClipEnable: BOOL(1),
        ScissorEnable: BOOL(0),
        MultisampleEnable: BOOL(1),
        AntialiasedLineEnable: BOOL(1),
    };

    const DEPTH_STENCIL_DESC: D3D10_DEPTH_STENCIL_DESC =
        D3D10_DEPTH_STENCIL_DESC {
            DepthEnable: BOOL(1),
            DepthWriteMask: D3D10_DEPTH_WRITE_MASK_ALL,
            DepthFunc: D3D10_COMPARISON_GREATER_EQUAL,
            StencilEnable: BOOL(1),
            StencilReadMask: D3D10_DEFAULT_STENCIL_READ_MASK as u8,
            StencilWriteMask: D3D10_DEFAULT_STENCIL_WRITE_MASK as u8,
            FrontFace: D3D10_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D10_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D10_STENCIL_OP_KEEP,
                StencilPassOp: D3D10_STENCIL_OP_REPLACE,
                StencilFunc: D3D10_COMPARISON_ALWAYS,
            },
            BackFace: D3D10_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D10_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D10_STENCIL_OP_KEEP,
                StencilPassOp: D3D10_STENCIL_OP_REPLACE,
                StencilFunc: D3D10_COMPARISON_ALWAYS,
            },
        };

    const SAMPLER_DESC: D3D10_SAMPLER_DESC = D3D10_SAMPLER_DESC {
        Filter: D3D10_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D10_TEXTURE_ADDRESS_BORDER,
        AddressV: D3D10_TEXTURE_ADDRESS_BORDER,
        AddressW: D3D10_TEXTURE_ADDRESS_BORDER,
        ComparisonFunc: D3D10_COMPARISON_ALWAYS,
        BorderColor: [1., 1., 1., 1.],
        ..self::zeroed()
    };

    const BLEND_DESC: D3D10_BLEND_DESC = D3D10_BLEND_DESC {
        AlphaToCoverageEnable: BOOL(0),
        BlendEnable: [
            BOOL(1),
            BOOL(0),
            BOOL(0),
            BOOL(0),
            BOOL(0),
            BOOL(0),
            BOOL(0),
            BOOL(0),
        ],
        SrcBlend: D3D10_BLEND_SRC_ALPHA,
        DestBlend: D3D10_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D10_BLEND_OP_ADD,
        SrcBlendAlpha: D3D10_BLEND_ONE,
        DestBlendAlpha: D3D10_BLEND_ZERO,
        BlendOpAlpha: D3D10_BLEND_OP_ADD,
        RenderTargetWriteMask: [
            D3D10_COLOR_WRITE_ENABLE_ALL.0 as _,
            self::zeroed(),
            self::zeroed(),
            self::zeroed(),
            self::zeroed(),
            self::zeroed(),
            self::zeroed(),
            self::zeroed(),
        ],
    };
}

impl Renderer {
    fn create_vertex_buffer(
        device: &ID3D10Device,
        data: &[VertexData],
    ) -> Result<ID3D10Buffer> {
        let mut vertex_buffer = None;
        unsafe {
            device.CreateBuffer(
                &D3D10_BUFFER_DESC {
                    ByteWidth: mem::size_of_val(data) as _,
                    Usage: D3D10_USAGE_IMMUTABLE,
                    BindFlags: D3D10_BIND_VERTEX_BUFFER.0 as _,
                    ..D3D10_BUFFER_DESC::default()
                },
                Some(&D3D10_SUBRESOURCE_DATA {
                    pSysMem: data.as_ptr() as _,
                    ..D3D10_SUBRESOURCE_DATA::default()
                }),
                Some(&mut vertex_buffer),
            )
        }?;
        Ok(vertex_buffer.unwrap())
    }

    fn create_index_buffer(
        device: &ID3D10Device,
        data: &[u32],
    ) -> Result<ID3D10Buffer> {
        let mut index_buffer = None;
        unsafe {
            device.CreateBuffer(
                &D3D10_BUFFER_DESC {
                    ByteWidth: mem::size_of_val(data) as _,
                    Usage: D3D10_USAGE_IMMUTABLE,
                    BindFlags: D3D10_BIND_INDEX_BUFFER.0 as _,
                    ..D3D10_BUFFER_DESC::default()
                },
                Some(&D3D10_SUBRESOURCE_DATA {
                    pSysMem: data.as_ptr() as _,
                    ..D3D10_SUBRESOURCE_DATA::default()
                }),
                Some(&mut index_buffer),
            )
        }?;
        Ok(index_buffer.unwrap())
    }

    fn get_render_target_size(
        rtv: &ID3D10RenderTargetView,
    ) -> Result<(u32, u32)> {
        let tex = unsafe { rtv.GetResource() }?.cast::<ID3D10Texture2D>()?;
        let mut desc = self::zeroed();
        unsafe { tex.GetDesc(&mut desc) };
        Ok((desc.Width, desc.Height))
    }
}

/// A custom state block to capture and restore the Direct3D10 pipeline state.
/// This is used as DXVK does not implement `ID3D10StateBlock`.
struct StateBlock {
    primitive_topology: D3D_PRIMITIVE_TOPOLOGY,
    input_layout: Option<ID3D10InputLayout>,
    vertex_shader: Option<ID3D10VertexShader>,
    pixel_shader: Option<ID3D10PixelShader>,
    rasterizer_state: Option<ID3D10RasterizerState>,
    viewports: Vec<D3D10_VIEWPORT>,
    sampler_states: Vec<Option<ID3D10SamplerState>>,
    render_targets: Vec<Option<ID3D10RenderTargetView>>,
    depth_stencil_view: Option<ID3D10DepthStencilView>,
    blend_state: Option<ID3D10BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,
    depth_stencil_state: Option<ID3D10DepthStencilState>,
    stencil_ref: u32,
}
impl StateBlock {
    unsafe fn new(device: &ID3D10Device) -> Result<Self> {
        let mut state_block = StateBlock {
            primitive_topology: D3D_PRIMITIVE_TOPOLOGY::default(),
            input_layout: None,
            vertex_shader: None,
            pixel_shader: None,
            rasterizer_state: None,
            viewports: Vec::new(),
            sampler_states: vec![None; 16],
            render_targets: vec![None; 8],
            depth_stencil_view: None,
            blend_state: None,
            blend_factor: [0.0; 4],
            sample_mask: 0,
            depth_stencil_state: None,
            stencil_ref: 0,
        };

        // Capture current state
        state_block.primitive_topology = device.IAGetPrimitiveTopology();
        state_block.input_layout = device.IAGetInputLayout().ok();
        state_block.vertex_shader = device.VSGetShader().ok();
        state_block.pixel_shader = device.PSGetShader().ok();
        state_block.rasterizer_state = device.RSGetState().ok();

        let mut num_viewports: u32 = 0;
        device.RSGetViewports(&mut num_viewports, None);
        state_block
            .viewports
            .resize(num_viewports as usize, D3D10_VIEWPORT::default());
        device.RSGetViewports(
            &mut num_viewports,
            Some(state_block.viewports.as_mut_ptr()),
        );

        device.PSGetSamplers(0, Some(&mut state_block.sampler_states));

        let mut depth_stencil_view = None;
        device.OMGetRenderTargets(
            Some(&mut state_block.render_targets),
            Some(&mut depth_stencil_view),
        );
        state_block.depth_stencil_view = depth_stencil_view;

        let mut blend_state = None;
        device.OMGetBlendState(
            Some(&mut blend_state),
            Some(&mut state_block.blend_factor),
            Some(&mut state_block.sample_mask),
        );
        state_block.blend_state = blend_state;

        let mut depth_stencil_state = None;
        device.OMGetDepthStencilState(
            Some(&mut depth_stencil_state),
            Some(&mut state_block.stencil_ref),
        );
        state_block.depth_stencil_state = depth_stencil_state;

        Ok(state_block)
    }
    unsafe fn apply(&self, device: &ID3D10Device) {
        device.IASetPrimitiveTopology(self.primitive_topology);
        device.IASetInputLayout(self.input_layout.as_ref());
        device.VSSetShader(self.vertex_shader.as_ref());
        device.PSSetShader(self.pixel_shader.as_ref());
        device.RSSetState(self.rasterizer_state.as_ref());
        device.RSSetViewports(Some(&self.viewports));
        device.PSSetSamplers(0, Some(&self.sampler_states));
        device.OMSetRenderTargets(
            Some(&self.render_targets),
            self.depth_stencil_view.as_ref(),
        );
        device.OMSetBlendState(
            self.blend_state.as_ref(),
            &self.blend_factor,
            self.sample_mask,
        );
        device.OMSetDepthStencilState(
            self.depth_stencil_state.as_ref(),
            self.stencil_ref,
        );
    }
}
