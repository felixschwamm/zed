use std::sync::Arc;

/// The GPU device and queue a window renders with.
///
/// Textures an application paints with [`gpui::Window::paint_external_texture`]
/// must be created on this device. Work submitted on this queue before the
/// window draws is ordered before the frame that samples it. After a device
/// loss GPUI renders with a new device, which `Arc::ptr_eq` on `device` detects.
#[derive(Clone)]
pub struct WgpuDevice {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub adapter_info: wgpu::AdapterInfo,
}

/// Access to the wgpu device of a window.
pub trait WgpuWindowExt {
    /// The device of a window drawn by the wgpu renderer, `None` for other renderers.
    fn wgpu_device(&self) -> Option<WgpuDevice>;
}

impl WgpuWindowExt for gpui::Window {
    fn wgpu_device(&self) -> Option<WgpuDevice> {
        self.gpu_device()?.downcast_ref::<WgpuDevice>().cloned()
    }
}

/// Wraps a view of an RGBA texture for [`gpui::Window::paint_external_texture`].
///
/// The renderer binds the view for the frames that draw it; wgpu keeps the
/// texture alive until the GPU has finished with those frames, so the
/// application may drop or replace it at any time.
///
/// The texture holds sRGB-encoded color in a non-sRGB format such as
/// `Rgba8Unorm`, the same encoding GPUI renders in, and is created with
/// `TEXTURE_BINDING` usage. Its alpha is straight, not premultiplied.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn external_texture(
    view: wgpu::TextureView,
    size: gpui::Size<gpui::DevicePixels>,
) -> gpui::ExternalTexture {
    gpui::ExternalTexture::new(Arc::new(view), size)
}
