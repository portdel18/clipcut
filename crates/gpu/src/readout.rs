//! Export read-back of the resolved 8-bit picture: RGBA8 as it is, or NV12 converted on the GPU
//! (`readout.wgsl`) for encoders that take it. Either is a plain copy on the CPU side, where the
//! float read-back ([`GpuCompositor::read_accumulator`](crate::GpuCompositor)) needs 8 bytes a
//! pixel, a half-float conversion and then the RGBA8 and YUV conversions.

/// The NV12 compute stage and the buffers it reuses from frame to frame.
pub(crate) struct Readout {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    /// (storage written by the shader, MAP_READ copy, size): NV12.
    nv12: Option<(wgpu::Buffer, wgpu::Buffer, u64)>,
    /// MAP_READ copy of the RGBA8 output (rows padded to 256 bytes), and its size.
    rgba: Option<(wgpu::Buffer, u64)>,
    dims: wgpu::Buffer,
}

impl Readout {
    /// `None` on a device without compute shaders (GLES 3.0 and the like): NV12 is then made on
    /// the CPU.
    pub(crate) fn new(device: &wgpu::Device) -> Option<Self> {
        if device.limits().max_compute_workgroups_per_dimension == 0 || device.limits().max_storage_buffers_per_shader_stage == 0 {
            return None;
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let made = Self::build(device);
        match crate::poll_now(scope.pop()) {
            Some(None) => Some(made),
            Some(Some(e)) => {
                log::warn!("GPU NV12 conversion unavailable on this device, converting on the CPU: {e}");
                None
            }
            None => None,
        }
    }

    fn build(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("readout"),
            source: wgpu::ShaderSource::Wgsl(include_str!("readout.wgsl").into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("readout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pl =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("readout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("readout"),
            layout: Some(&pl),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let dims = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readout-dims"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { pipeline, bgl, nv12: None, rgba: None, dims }
    }

    /// Record the NV12 conversion of `src` (`w` × `h`, `w` a multiple of 4 and `h` even) and its
    /// copy into the read-back buffer; returns that buffer's byte count.
    pub(crate) fn record_nv12(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        w: u32,
        h: u32,
    ) -> Option<u64> {
        if w == 0 || h == 0 || !w.is_multiple_of(4) || !h.is_multiple_of(2) {
            return None;
        }
        let size = u64::from(w).checked_mul(u64::from(h))?.checked_mul(3)? / 2;
        if self.nv12.as_ref().is_none_or(|(_, _, s)| *s != size) {
            let buf = |label: &str, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            self.nv12 = Some((
                buf("readout-nv12", wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC),
                buf("readout-nv12-read", wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ),
                size,
            ));
        }
        let (storage, read, _) = self.nv12.as_ref()?;
        let mut d = [0u8; 16];
        d[..4].copy_from_slice(&w.to_le_bytes());
        d[4..8].copy_from_slice(&h.to_le_bytes());
        queue.write_buffer(&self.dims, 0, &d);
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("readout"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: storage.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.dims.as_entire_binding() },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("readout"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups((w / 4).div_ceil(8), (h / 2).div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(storage, 0, read, 0, size);
        Some(size)
    }

    /// Record the copy of the RGBA8 output `tex` into the read-back buffer; returns the row pitch.
    pub(crate) fn record_rgba(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, tex: &wgpu::Texture, w: u32, h: u32) -> Option<u32> {
        let row = w.checked_mul(4)?.div_ceil(256).checked_mul(256)?;
        let size = u64::from(row) * u64::from(h);
        if self.rgba.as_ref().is_none_or(|(_, s)| *s < size) {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readout-rgba-read"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.rgba = Some((b, size));
        }
        let (buf, _) = self.rgba.as_ref()?;
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        Some(row)
    }

    /// The NV12 bytes recorded by [`Readout::record_nv12`], once the GPU has made them.
    pub(crate) fn take_nv12(&self, device: &wgpu::Device) -> Option<Vec<u8>> {
        let (_, read, size) = self.nv12.as_ref()?;
        let out = map(device, read, |data| data.get(..usize::try_from(*size).ok()?).map(<[u8]>::to_vec));
        out.flatten()
    }

    /// The RGBA8 rows recorded by [`Readout::record_rgba`] without their padding.
    pub(crate) fn take_rgba(&self, device: &wgpu::Device, w: u32, h: u32, row: u32) -> Option<Vec<u8>> {
        let (buf, _) = self.rgba.as_ref()?;
        let (w, h, row) = (w as usize, h as usize, row as usize);
        map(device, buf, |data| {
            let mut out = Vec::with_capacity(w.checked_mul(h)?.checked_mul(4)?);
            for y in 0..h {
                out.extend_from_slice(data.get(y * row..y * row + w * 4)?);
            }
            Some(out)
        })
        .flatten()
    }
}

/// Map `buf` for reading, run `f` on its bytes and unmap it again.
fn map<R>(device: &wgpu::Device, buf: &wgpu::Buffer, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
    let t = std::time::Instant::now();
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    crate::wait_mapped(device)?;
    rx.recv().ok()?.ok()?;
    crate::export_renderer::timing::add_map_wait(t.elapsed());
    let t = std::time::Instant::now();
    let data = slice.get_mapped_range().ok()?;
    let r = f(&data);
    drop(data);
    buf.unmap();
    crate::export_renderer::timing::add_convert(t.elapsed());
    Some(r)
}
