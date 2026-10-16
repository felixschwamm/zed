//! A `wgpu_canvas` drawing many triangles with its own pipeline, between ordinary elements.
//! Run with `cargo run -p gpui --example wgpu_canvas --features wgpu`.

#[path = "example_support/fonts.rs"]
mod example_support;

use gpui::{App, Bounds, Context, Window, WindowBounds, WindowOptions, WGPU_CANVAS_FORMAT, WgpuCanvasFrame, div, prelude::*, px, rgb, size, wgpu_canvas};
use gpui_platform::application;
use std::cell::RefCell;
use std::rc::Rc;

const TRIANGLES: u32 = 20_000;

const SHADER: &str = r#"
@vertex
fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    // A grid of small triangles, each with its own color.
    let columns = 200u;
    let cell = vec2<f32>(f32(instance % columns), f32(instance / columns));
    let corner = array<vec2<f32>, 3>(vec2<f32>(0.1, 0.9), vec2<f32>(0.9, 0.9), vec2<f32>(0.5, 0.1))[vertex];
    let position = (cell + corner) / vec2<f32>(f32(columns), 100.0) * 2.0 - 1.0;
    var out: VertexOut;
    out.position = vec4<f32>(position.x, -position.y, 0.0, 1.0);
    out.color = vec4<f32>(cell.x / f32(columns), cell.y / 100.0, 0.6, 1.0);
    return out;
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@fragment
fn fragment(input: VertexOut) -> @location(0) vec4<f32> {
    return input.color;
}
"#;

struct Triangles {
    pipeline: Rc<RefCell<Option<wgpu::RenderPipeline>>>,
}

fn paint(pipeline: &RefCell<Option<wgpu::RenderPipeline>>, frame: &mut WgpuCanvasFrame<'_>) {
    if !frame.target_is_new {
        return;
    }
    let mut pipeline = pipeline.borrow_mut();
    let pipeline = pipeline.get_or_insert_with(|| {
        let module = frame.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("triangles"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        frame.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("triangles"),
            layout: None,
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fragment"), compilation_options: Default::default(), targets: &[Some(WGPU_CANVAS_FORMAT.into())] }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    });
    let mut pass = frame.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("triangles"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: frame.target_view,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.06, g: 0.08, b: 0.1, a: 1.0 }), store: wgpu::StoreOp::Store },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.draw(0..3, 0..TRIANGLES);
}

impl Render for Triangles {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let pipeline = self.pipeline.clone();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0xf4f4f0))
            .p_4()
            .gap_2()
            .child(div().text_color(rgb(0x202020)).child(format!("{TRIANGLES} triangles in a wgpu_canvas")))
            .child(wgpu_canvas("triangles", move |frame| paint(&pipeline, frame)).flex_1().w_full().rounded_md())
            .child(div().h_8().w_full().bg(rgb(0x0b5f86)).rounded_md())
    }
}

fn main() {
    application().run(|cx: &mut App| {
        if !example_support::load_fonts(cx) {
            return;
        }
        let bounds = Bounds::centered(None, size(px(800.), px(600.0)), cx);
        cx.open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() }, |_, cx| cx.new(|_| Triangles { pipeline: Rc::new(RefCell::new(None)) }))
            .expect("opening the window");
        cx.activate(true);
    });
}
