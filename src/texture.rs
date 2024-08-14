// This file contains implementations inspired by or derived from the following
// sources:
// - https://github.com/ohchase/egui-directx/blob/master/egui-directx11/src/texture.rs
//
// Here I would express my gratitude for their contributions to the Rust
// community. Their work served as a valuable reference and inspiration for this
// project.
//
// Nekomaru, March 2024

use std::{collections::HashMap, mem};

use egui::{Color32, ImageData, TextureId, TexturesDelta};

use windows::{
    core::Result,
    Win32::Graphics::{Direct3D10::*, Dxgi::Common::*},
};

struct Texture {
    tex: ID3D10Texture2D,
    srv: ID3D10ShaderResourceView,
    pixels: Vec<Color32>,
    width: usize,
}

pub struct TexturePool {
    device: ID3D10Device,
    pool: HashMap<TextureId, Texture>,
}

impl TexturePool {
    pub fn new(device: &ID3D10Device) -> Self {
        Self {
            device: device.clone(),
            pool: HashMap::new(),
        }
    }

    pub fn get_srv(&self, tid: TextureId) -> Option<ID3D10ShaderResourceView> {
        self.pool.get(&tid).map(|t| t.srv.clone())
    }

    pub fn update(
        &mut self,
        ctx: &ID3D10Device,
        delta: TexturesDelta,
    ) -> Result<()> {
        for (tid, delta) in delta.set {
            if delta.is_whole() {
                self.pool.insert(
                    tid,
                    Self::create_texture(&self.device, delta.image)?,
                );
                // the old texture is returned and dropped here, freeing
                // all its gpu resource.
            } else if let Some(tex) = self.pool.get_mut(&tid) {
                Self::update_partial(
                    ctx,
                    tex,
                    delta.image,
                    delta.pos.unwrap(),
                )?;
            } else {
                log::warn!("egui wants to update a non-existing texture {tid:?}. this request will be ignored.");
            }
        }
        for tid in delta.free {
            self.pool.remove(&tid);
        }
        Ok(())
    }

    fn update_partial(
        ctx: &ID3D10Device,
        old: &mut Texture,
        image: ImageData,
        [nx, ny]: [usize; 2],
    ) -> Result<()> {
        match image {
            ImageData::Font(f) => {
                let row_pitch = old.width * 4; // 4 bytes per pixel
                let mut update_data = vec![0u8; f.height() * row_pitch];

                for y in 0..f.height() {
                    for x in 0..f.width() {
                        let frac = y * f.width() + x;
                        let whole = (ny + y) * old.width + nx + x;
                        let dst_idx = y * row_pitch + x * 4;

                        // Create new Color32 and update old.pixels
                        let new_color = Color32::from_rgba_premultiplied(
                            255,
                            255,
                            255,
                            (f.pixels[frac] * 255.) as u8,
                        );
                        old.pixels[whole] = new_color;

                        // Update update_data
                        let color_array = new_color.to_array();
                        update_data[dst_idx..dst_idx + 4]
                            .copy_from_slice(&color_array);
                    }
                }

                let subresource_data = D3D10_BOX {
                    left: nx as u32,
                    top: ny as u32,
                    front: 0,
                    right: (nx + f.width()) as u32,
                    bottom: (ny + f.height()) as u32,
                    back: 1,
                };

                unsafe {
                    ctx.UpdateSubresource(
                        &old.tex,
                        0,
                        Some(&subresource_data),
                        update_data.as_ptr() as _,
                        row_pitch as u32,
                        0,
                    );
                }
            },
            _ => unreachable!(),
        }
        Ok(())
    }

    fn create_texture(
        device: &ID3D10Device,
        data: ImageData,
    ) -> Result<Texture> {
        let width = data.width();

        let pixels = match &data {
            ImageData::Color(c) => c.pixels.clone(),
            ImageData::Font(f) => f
                .pixels
                .iter()
                .map(|a| {
                    Color32::from_rgba_premultiplied(
                        255,
                        255,
                        255,
                        (a * 255.) as u8,
                    )
                })
                .collect(),
        };

        let desc = D3D10_TEXTURE2D_DESC {
            Width: data.width() as _,
            Height: data.height() as _,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D10_USAGE_DYNAMIC,
            BindFlags: D3D10_BIND_SHADER_RESOURCE.0 as _,
            CPUAccessFlags: D3D10_CPU_ACCESS_WRITE.0 as _,
            ..Default::default()
        };

        let subresource_data = D3D10_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as _,
            SysMemPitch: (width * mem::size_of::<Color32>()) as u32,
            SysMemSlicePitch: 0,
        };

        let tex =
            unsafe { device.CreateTexture2D(&desc, Some(&subresource_data)) }?;

        let mut srv = None;
        unsafe { device.CreateShaderResourceView(&tex, None, Some(&mut srv)) }?;
        let srv = srv.unwrap();

        Ok(Texture {
            tex,
            srv,
            width,
            pixels,
        })
    }
}
