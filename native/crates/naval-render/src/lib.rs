//! The native client's renderer core.
//!
//! A frame is explicit passes over owned targets: layers prepare, then draw into
//! a linear HDR scene target (`Rgba16Float`) with reversed depth
//! (`Depth32Float`, cleared to 0, `GreaterEqual`), then the display pass grades
//! it with the game's look and three.js's AgX onto an sRGB output. See
//! [`frame`] for the order and [`SceneLayer`] for how another crate draws.
//!
//! Portable: this crate builds for `wasm32-unknown-unknown`. It never touches
//! the file system, blocks on the GPU or spawns threads; the `hot-reload`
//! feature (native only) adds file polling for shaders.

pub mod aerial;
pub mod camera;
pub mod contact;
pub mod copies;
pub mod cover;
pub mod debug_view;
pub mod device_cache;
pub mod finish;
pub mod frame;
#[cfg(not(target_arch = "wasm32"))]
pub mod hot_reload;
pub mod lights;
pub mod palette;
pub mod photo;
pub mod pipelines;
pub mod post;
pub mod relay;
pub mod resources;
pub mod shaders;
pub mod shadows;
pub mod test_scene;
pub mod timing;
pub mod upload;

// The post and photo modules at their old paths (`naval_render::taa`, `crate::photo_optics`).
pub use photo::{film, look_preview, photo_optics, play_camera, shutter};
pub use post::{antialiasing, display, dither, present, shimmer, sun_glare, taa};

pub use aerial::{
    AERIAL_FAR, AERIAL_FORMAT, AERIAL_NEAR, AERIAL_SIZE, AERIAL_SKY_BYTES, AERIAL_VOLUME_BYTES,
    AerialVolume, aerial_slice_distance,
};
pub use antialiasing::{Antialiasing, POST_FRAME, RENDER_SCALE_MIN, capped_scale, scene_size};
pub use camera::{Camera, CameraMotion, CameraUniforms, FrameTime};
pub use contact::{CONTACT_DEPTH, CONTACT_FORMAT, CONTACT_MASK};
pub use copies::{
    CopyPoint, SCENE_BACKDROP_COLOR, SCENE_BACKDROP_DEPTH, SCENE_OPAQUE_COLOR, SCENE_OPAQUE_DEPTH,
    SceneCopies,
};
pub use cover::{Cover, CoverTarget};
pub use debug_view::DebugView;
pub use device_cache::DeviceCache;
pub use display::{CAMERA_EXPOSURE, DisplaySettings};
pub use dither::dither_levels;
pub use finish::{Finisher, Part};
pub use frame::{
    DEPTH_CLEAR, DEPTH_COMPARE, DEPTH_FORMAT, DrawContext, FrameInput, FrameReport, HDR_FORMAT,
    OverlayContext, OverlayPipeline, OverlayPipelineId, Phase, PrepareContext, ReloadReport,
    RenderContext, RenderOutput, Renderer, SCENE_DEPTH, SCENE_HDR, SceneLayer, ScenePipeline,
    SceneView, SecondView, VertexLayout, extended_range, scene_depth_state,
};
pub use lights::{MAX_POINT_LIGHTS, PointLight, PointLights, SceneFog};
pub use look_preview::LookPreviews;
pub use photo::{DepthOfField, Film as LookFilm, Look as PhotoLook, PhotoPost};
pub use photo_optics::{Coating as PhotoCoating, FlareSun, Optics as PhotoOptics};
pub use pipelines::{
    LABEL_FORBIDDEN, MakePipelines, Pipeline, PipelineBuild, PipelineId, PipelineRecipe,
    PipelineRegistry, PipelineStatus, PlaceholderPipelines, RecipeFn, default_build_threads,
    portable_label,
};
pub use play_camera::PlayCamera;
pub use post::{
    BLOOM_GLARES, BloomGlare, BloomSettings, EYE_STATE_BUFFER, ExposureControl,
    EyeAdaptationSettings, SKY_MEAN_BUFFER, SkyMean, compute_recipe, fullscreen_recipe,
};
pub use present::LinearPresent;
pub use resources::{Lifetime, ResourceStats, Resources, TextureDesc, ViewTextures};
pub use shaders::{ShaderDefValue, ShaderDefs, ShaderError, ShaderFile, ShaderLibrary};
pub use shadows::{
    CasterPipeline, CasterSphere, HullBox, LandView, SHADOW_FORMAT, SUN_OCCLUDER_SIZE, ShadowMap,
    ShadowMapKind, ShadowSettings, Sun, SunOccluderUniform, SunOccluders,
};
pub use shimmer::Shimmer;
pub use shutter::{MovingBody, MovingShape, Shutter};
pub use taa::{DISPLAY_FRAME, MotionPipeline, SCENE_MOTION, TAA_RESOLVED, TaaSettings};
pub use test_scene::TestScene;
pub use timing::{FrameTimings, GpuTimer, PassTiming};
pub use upload::{PassUploads, UploadSink, UploadStats, Uploads};
