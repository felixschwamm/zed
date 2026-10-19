//! Renders an animated texture on the window's own wgpu device every frame and
//! paints it next to GPUI's quads: once at full size, once clipped by a
//! scrolling container. The swatch row compares a color written by the shader
//! with the same color painted by GPUI; the two halves must match.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod example {
    use gpui::{
        App, Bounds, Context, DevicePixels, Pixels, Window, WindowBounds, WindowOptions, canvas,
        div, prelude::*, px, rgb, size,
    };
    use gpui_wgpu::{WgpuDevice, WgpuWindowExt as _, external_texture, wgpu};
    use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

    const SHADER: &str = r#"
struct Params { time: f32, width: f32, height: f32, pad: f32 }
@group(0) @binding(0) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = position.xy / vec2<f32>(params.width, params.height);
    // The bottom strip is the swatch: #6d4bf2 written as sRGB-encoded bytes.
    if (uv.y > 0.85) {
        return vec4<f32>(109.0 / 255.0, 75.0 / 255.0, 242.0 / 255.0, 1.0);
    }
    let wave = 0.5 + 0.5 * sin(uv.x * 12.0 + params.time * 2.0);
    return vec4<f32>(uv.x, wave * uv.y, 1.0 - uv.x, 1.0);
}
"#;

    struct Target {
        size: gpui::Size<DevicePixels>,
        view: wgpu::TextureView,
    }

    /// The pipeline and the texture it renders into, kept per device so that a
    /// device loss rebuilds them.
    struct Producer {
        device: WgpuDevice,
        pipeline: wgpu::RenderPipeline,
        layout: wgpu::BindGroupLayout,
        params: wgpu::Buffer,
        target: Option<Target>,
    }

    impl Producer {
        fn new(device: WgpuDevice) -> Self {
            let module = device
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("external_texture_example"),
                    source: wgpu::ShaderSource::Wgsl(SHADER.into()),
                });
            let layout = device
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
            let pipeline_layout =
                device
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: None,
                        bind_group_layouts: &[Some(&layout)],
                        immediate_size: 0,
                    });
            let pipeline = device
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("external_texture_example"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs"),
                        targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                        compilation_options: Default::default(),
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                });
            let params = device.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            Self {
                device,
                pipeline,
                layout,
                params,
                target: None,
            }
        }

        fn render(&mut self, size: gpui::Size<DevicePixels>, time: f32) -> gpui::ExternalTexture {
            if self
                .target
                .as_ref()
                .is_none_or(|target| target.size != size)
            {
                let texture = self.device.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("external_texture_example"),
                    size: wgpu::Extent3d {
                        width: size.width.0 as u32,
                        height: size.height.0 as u32,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                self.target = Some(Target {
                    size,
                    view: texture.create_view(&Default::default()),
                });
            }
            let target = self.target.as_ref().expect("created above");
            let params = [time, size.width.0 as f32, size.height.0 as f32, 0.0];
            self.device
                .queue
                .write_buffer(&self.params, 0, bytemuck::cast_slice(&params));
            let bind_group = self
                .device
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params.as_entire_binding(),
                    }],
                });
            let mut encoder = self
                .device
                .device
                .create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            self.device.queue.submit([encoder.finish()]);
            external_texture(target.view.clone(), size)
        }
    }

    struct Showcase {
        start: Instant,
        producers: [Rc<RefCell<Option<Producer>>>; 2],
    }

    /// Renders into the producer at the element's size and paints the result.
    fn animated_texture(producer: Rc<RefCell<Option<Producer>>>, time: f32) -> impl IntoElement {
        canvas(
            move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
                let device = window.wgpu_device()?;
                let mut producer = producer.borrow_mut();
                if producer
                    .as_ref()
                    .is_none_or(|current| !Arc::ptr_eq(&current.device.device, &device.device))
                {
                    *producer = Some(Producer::new(device));
                }
                let size = bounds.size.to_device_pixels(window.scale_factor());
                (size.width.0 > 0 && size.height.0 > 0)
                    .then(|| producer.as_mut().expect("set above").render(size, time))
            },
            |bounds, texture, window, _| {
                if let Some(texture) = texture {
                    window.paint_external_texture(bounds, texture);
                }
            },
        )
        .size_full()
    }

    impl Render for Showcase {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            window.request_animation_frame();
            let time = self.start.elapsed().as_secs_f32();
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .bg(rgb(0x0e0c15))
                .text_color(rgb(0xf4f2f8))
                .child("A texture rendered by the application, composited by GPUI")
                .child(
                    div()
                        .h(px(240.))
                        .w_full()
                        .rounded_lg()
                        .border_2()
                        .border_color(rgb(0x6d4bf2))
                        .p_1()
                        .child(animated_texture(self.producers[0].clone(), time)),
                )
                .child("Swatch: shader (left) and GPUI (right) must match")
                .child(
                    div()
                        .flex()
                        .gap_0()
                        .child(div().w(px(120.)).h(px(40.)).bg(rgb(0x6d4bf2)))
                        .child(div().w(px(120.)).h(px(40.)).bg(rgb(0x6d4bf2))),
                )
                .child("Clipped by a scrolling container")
                .child(
                    div()
                        .id("scroller")
                        .h(px(160.))
                        .w(px(360.))
                        .overflow_y_scroll()
                        .border_1()
                        .border_color(rgb(0x2c2740))
                        .child(div().h(px(80.)).child("scroll down"))
                        .child(
                            div()
                                .h(px(300.))
                                .w_full()
                                .child(animated_texture(self.producers[1].clone(), time)),
                        )
                        .child(div().h(px(80.))),
                )
        }
    }

    pub fn main() {
        gpui_platform::application().run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(720.), px(720.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Showcase {
                        start: Instant::now(),
                        producers: Default::default(),
                    })
                },
            )
            .expect("open window");
            cx.activate(true);
        });
    }
}

fn main() {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    example::main();
}
