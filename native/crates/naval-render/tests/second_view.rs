//! A second view of the scene in the same frame (`Renderer::render_view`, native/docs/render-core/frame.md "A second
//! view in the same frame"): its viewport textures are its own size and kept apart from the main view's, which are
//! neither remade nor resized by it, and the layers are told which view they prepare for.

use std::cell::RefCell;
use std::rc::Rc;

use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    Camera, DrawContext, FrameInput, FrameTime, Lifetime, Phase, PrepareContext, RenderOutput,
    Renderer, Resources, SCENE_DEPTH, SCENE_HDR, SceneLayer, SceneView, SecondView, TextureDesc,
    ViewTextures,
};

const MAIN: (u32, u32) = (64, 32);
const SECOND: (u32, u32) = (24, 16);
/// The second view's box resized (a HUD scale changed with the panel up).
const RESIZED: (u32, u32) = (40, 20);

/// Each prepare's view and viewport.
type Seen = Rc<RefCell<Vec<(SceneView, (u32, u32))>>>;

/// Records the view it was told and the viewport each prepare saw.
struct Witness {
    view: SceneView,
    seen: Seen,
}

impl SceneLayer for Witness {
    fn label(&self) -> &str {
        "witness"
    }

    fn set_view(&mut self, view: SceneView) {
        self.view = view;
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        // Every viewport texture at the size of the view being drawn: a smaller depth than colour fails a pass.
        let depth = ctx.resources.texture(SCENE_DEPTH).unwrap().desc.size;
        assert_eq!(depth, ctx.viewport, "{:?}", self.view);
        self.seen.borrow_mut().push((self.view, ctx.viewport));
    }

    fn draw(&self, _phase: Phase, _pass: &mut wgpu::RenderPass<'_>, _ctx: &DrawContext<'_>) {}
}

#[test]
fn a_swapped_view_holds_its_own_viewport_textures_and_gives_the_main_ones_back() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut resources = Resources::new(MAIN.0, MAIN.1);
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let desc = TextureDesc::viewport(wgpu::TextureFormat::Rgba8Unorm, usage, 1.0);
    let main_version = resources
        .create_texture(&gpu.device, "frame", desc.clone())
        .version;
    resources.create_texture(
        &gpu.device,
        "kept",
        TextureDesc::fixed(
            wgpu::TextureFormat::Rgba8Unorm,
            usage,
            4,
            4,
            Lifetime::Persistent,
        ),
    );
    let mut view = ViewTextures::default();
    view.set_viewport(SECOND.0, SECOND.1);
    resources.swap_view(&gpu.device, &mut view);
    // The view finds the main view's viewport texture made at its own size; other lifetimes stay as they are.
    assert_eq!(resources.viewport(), SECOND);
    assert_eq!(resources.texture("frame").unwrap().desc.size, SECOND);
    assert_ne!(resources.texture("frame").unwrap().version, main_version);
    assert_eq!(resources.texture("kept").unwrap().desc.size, (4, 4));
    // One the view registers is there for the main view too, at the main view's size.
    resources.create_texture(&gpu.device, "copy", desc);
    let view_version = resources.texture("frame").unwrap().version;
    resources.swap_view(&gpu.device, &mut view);
    assert_eq!(resources.viewport(), MAIN);
    assert_eq!(resources.texture("frame").unwrap().version, main_version);
    assert_eq!(resources.texture("frame").unwrap().desc.size, MAIN);
    assert_eq!(resources.texture("copy").unwrap().desc.size, MAIN);
    // And the view's own, kept for its next swap.
    resources.swap_view(&gpu.device, &mut view);
    assert_eq!(resources.texture("frame").unwrap().version, view_version);
}

#[test]
fn a_second_view_draws_at_its_size_and_leaves_the_main_view_as_it_was() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, MAIN.0, MAIN.1, CAPTURE_FORMAT).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut layer = Witness {
        view: SceneView::Main,
        seen: seen.clone(),
    };
    let main = Capture::new(&gpu, MAIN.0, MAIN.1);
    let second = Capture::new(&gpu, SECOND.0, SECOND.1);
    let mut view = SecondView::default();
    let frame = |renderer: &mut Renderer, layer: &mut Witness, index: u64| {
        let time = FrameTime {
            frame: index,
            dt: 1.0 / 60.0,
            ..Default::default()
        };
        renderer
            .render(
                FrameInput {
                    camera: &Camera::default(),
                    time,
                },
                &mut [layer as &mut dyn SceneLayer],
                RenderOutput {
                    view: &main.view,
                    format: CAPTURE_FORMAT,
                    width: MAIN.0,
                    height: MAIN.1,
                },
            )
            .unwrap();
    };
    frame(&mut renderer, &mut layer, 1);
    let hdr = renderer.resources().texture(SCENE_HDR).unwrap().version;
    layer.set_view(SceneView::Second);
    renderer
        .render_view(
            &mut view,
            FrameInput {
                camera: &Camera::look_at(glam::Vec3::new(0.0, 30.0, 600.0), glam::Vec3::ZERO),
                time: FrameTime {
                    frame: 1,
                    ..Default::default()
                },
            },
            &mut [&mut layer as &mut dyn SceneLayer],
            RenderOutput {
                view: &second.view,
                format: CAPTURE_FORMAT,
                width: SECOND.0,
                height: SECOND.1,
            },
        )
        .unwrap();
    layer.set_view(SceneView::Main);
    assert_eq!(renderer.resources().viewport(), MAIN);
    assert_eq!(
        renderer.resources().texture(SCENE_HDR).unwrap().version,
        hdr
    );
    frame(&mut renderer, &mut layer, 2);
    // The main view's scene target was neither remade nor resized by the second view between its frames.
    assert_eq!(
        renderer.resources().texture(SCENE_HDR).unwrap().version,
        hdr
    );
    // The second view's box resized: its own textures are remade at the new size.
    let resized = Capture::new(&gpu, RESIZED.0, RESIZED.1);
    layer.set_view(SceneView::Second);
    renderer
        .render_view(
            &mut view,
            FrameInput {
                camera: &Camera::look_at(glam::Vec3::new(0.0, 30.0, 600.0), glam::Vec3::ZERO),
                time: FrameTime {
                    frame: 2,
                    ..Default::default()
                },
            },
            &mut [&mut layer as &mut dyn SceneLayer],
            RenderOutput {
                view: &resized.view,
                format: CAPTURE_FORMAT,
                width: RESIZED.0,
                height: RESIZED.1,
            },
        )
        .unwrap();
    layer.set_view(SceneView::Main);
    assert_eq!(renderer.resources().viewport(), MAIN);
    assert_eq!(
        *seen.borrow(),
        vec![
            (SceneView::Main, MAIN),
            (SceneView::Second, SECOND),
            (SceneView::Main, MAIN),
            (SceneView::Second, RESIZED),
        ]
    );
}
