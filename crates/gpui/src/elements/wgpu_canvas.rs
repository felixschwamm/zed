use crate::{
    App, Bounds, DevicePixels, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Size, Style, StyleRefinement, Styled, Window,
};
use refineable::Refineable;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

/// The texture format of a [`wgpu_canvas`] render target.
///
/// The texture is composited with straight (non-premultiplied) alpha.
pub const WGPU_CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The callback that records a [`wgpu_canvas`]'s content.
pub type WgpuCanvasPaint = Rc<dyn Fn(&mut WgpuCanvasFrame<'_>)>;

/// Everything a [`wgpu_canvas`] paint callback needs to render one frame.
///
/// The callback records commands into `encoder` that render into `target`. The renderer
/// submits the encoder together with the rest of the window's frame and then composites
/// the texture at the canvas bounds, so the canvas is ordered, clipped and layered like
/// any other element.
pub struct WgpuCanvasFrame<'a> {
    /// The renderer's device. It changes after GPU device recovery, so resources created
    /// from an earlier device must be recreated.
    pub device: &'a wgpu::Device,
    /// The renderer's queue.
    pub queue: &'a wgpu::Queue,
    /// The encoder of the window's frame.
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The render target, sized to the canvas bounds in device pixels and in
    /// [`WGPU_CANVAS_FORMAT`].
    pub target: &'a wgpu::Texture,
    /// A default view of `target`.
    pub target_view: &'a wgpu::TextureView,
    /// The size of `target`.
    pub size: Size<DevicePixels>,
    /// The window's scale factor.
    pub scale_factor: f32,
    /// Whether `target` was just created, in which case it holds no content yet. When it is
    /// false, the target still holds what the callback rendered into it in the previous
    /// frame, so a callback whose inputs did not change may return without recording.
    pub target_is_new: bool,
}

/// Creates an element whose content is rendered by `paint` with the window renderer's
/// wgpu device.
///
/// `id` identifies the canvas across frames so its render target can be reused; it must
/// be unique among the canvases of a window. Renderers that don't use wgpu (Metal,
/// DirectX) leave the canvas empty.
pub fn wgpu_canvas(
    id: impl Into<ElementId>,
    paint: impl Fn(&mut WgpuCanvasFrame<'_>) + 'static,
) -> WgpuCanvas {
    WgpuCanvas {
        id: id.into(),
        paint: Rc::new(paint),
        style: StyleRefinement::default(),
    }
}

/// An element rendered with wgpu. See [`wgpu_canvas`].
pub struct WgpuCanvas {
    id: ElementId,
    paint: WgpuCanvasPaint,
    style: StyleRefinement,
}

impl IntoElement for WgpuCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WgpuCanvas {
    type RequestLayoutState = Style;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let mut hasher = DefaultHasher::new();
        global_id.hash(&mut hasher);
        let corner_radii = style.corner_radii.to_pixels(window.rem_size());
        window.paint_wgpu_canvas(bounds, corner_radii, hasher.finish(), self.paint.clone());
    }
}

impl Styled for WgpuCanvas {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
