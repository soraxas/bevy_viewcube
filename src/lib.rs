//! A **view cube**: a 3D orientation gizmo for Bevy editors.
//!
//! A small cube rendered in a corner of the screen that mirrors a target
//! camera's orientation, with clickable faces, edges and corners that snap the
//! camera to canonical views — top / front / side / iso. Conventional in CAD
//! and 3D editors (Blender, Plasticity, Fusion, AutoCAD).
//!
//! # Camera-agnostic
//!
//! The cube never calls a camera controller's API. It reads the target's
//! `Transform` / `Projection` (to mirror it and to compute views) and emits
//! [`CameraRequest`] messages — look to a view, orbit by a drag, switch
//! projection, fit, align the up axis. A **driver** carries them out:
//!
//! - [`driver::BasicDriverPlugin`]: dependency-free, works on any plain
//!   `Camera3d`.
//! - [`driver::EditorCamDriverPlugin`] (feature `editor_cam`, on by default):
//!   adapts `bevy_editor_cam`'s `EditorCam`.
//! - your own: read [`CameraRequest`] and move your camera.
//!
//! # Wiring
//!
//! 1. Add [`ViewCubePlugin`] and a driver plugin.
//! 2. Add [`ViewCubeTarget`] to the camera the cube mirrors and drives.
//! 3. Keep the [`ViewCubeConfig`] resource current: whether the cube is
//!    `active` and the rectangle (`origin` + `panel_size`, physical px) it pins
//!    its viewport to.
//!
//! Options fixed when the app is built (style, render layer, font, labels,
//! axes placement) are [`ViewCubeSettings`] on the plugin; everything the host
//! updates at runtime lives in [`ViewCubeConfig`] and [`ViewCubeFrames`]. The
//! cube renders on its own render layer through a dedicated overlay camera, so
//! it never interacts with the host scene.
//!
//! ```no_run
//! use bevy::prelude::*;
//! use bevy_viewcube::{driver::BasicDriverPlugin, ViewCubePlugin, ViewCubeTarget};
//!
//! fn main() {
//!     App::new()
//!         .add_plugins((DefaultPlugins, ViewCubePlugin::default(), BasicDriverPlugin::default()))
//!         .add_systems(Startup, |mut commands: Commands| {
//!             commands.spawn((Camera3d::default(), ViewCubeTarget));
//!         })
//!         .run();
//! }
//! ```

use bevy::prelude::*;


mod cad;
mod cube;
pub mod driver;
mod fit;
mod systems;
mod text3d;
#[cfg(feature = "ui")]
mod ui;

pub use cube::{CubeAxisLabel, CubeCamera, CubeFace, CubeHover, CubePivot, ViewPreset};
pub use driver::{CameraAction, CameraRequest, ViewCubeFocus};
pub use fit::FitTarget;

/// System set holding every system of the cube's core (input handling, view
/// math, mirroring). Drivers run after it so a [`CameraRequest`] is carried out
/// the frame it's made; order your own systems against it too.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ViewCubeSet;

/// Marker the host adds to the camera the cube mirrors and drives. The camera
/// must be a root entity (no parent) and be controlled by a driver (see
/// [`driver`]).
#[derive(Component, Debug, Default)]
#[require(ViewCubeFocus)]
pub struct ViewCubeTarget;

/// Which screen corner the cube viewport pins to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CubeCorner {
    /// Top-left of the host rect.
    #[default]
    TopLeft,
    /// Top-right of the host rect.
    TopRight,
    /// Bottom-left of the host rect.
    BottomLeft,
    /// Bottom-right of the host rect.
    BottomRight,
}

/// Where the cube draws the active frame's local X / Y / Z axes. Since the
/// cube is aligned to the frame, they are the cube's own axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxesPlacement {
    /// No axes.
    Hidden,
    /// A small triad pinned to a corner of the cube viewport, like a UCS icon.
    /// It turns with the cube but takes no room from it.
    ViewportCorner(CubeCorner),
    /// Axes running along the cube's front-left-bottom corner, larger and
    /// attached to the cube.
    CubeCorner,
}

/// Host-owned configuration the cube reads each frame.
///
/// The host updates `active` and the `origin`/`panel_size` rect (physical px)
/// to follow whatever panel the cube should overlay. Every field here may
/// change at any time; options fixed at startup are in [`ViewCubeSettings`].
#[derive(Resource, Debug, Clone, Copy)]
pub struct ViewCubeConfig {
    /// When `false`, the cube camera is deactivated and the cube hidden.
    pub active: bool,
    /// Top-left of the host rectangle the cube pins inside (physical px).
    pub origin: UVec2,
    /// Size of the host rectangle (physical px). Used for corner placement.
    pub panel_size: UVec2,
    /// Corner of the host rect the cube sits in.
    pub corner: CubeCorner,
    /// Side length of the square cube viewport (logical px, DPI-scaled).
    pub viewport_px: u32,
    /// Inset from the chosen corner (logical px, DPI-scaled).
    pub inset_px: u32,
    /// Multiplier on pointer movement when dragging the cube to orbit the
    /// target camera.
    pub drag_sensitivity: f32,
    /// Double-clicking the home button emits [`ToggleProjection`]. Disable if the host
    /// binds double-click itself; hosts can still write the message directly.
    pub double_click_projection: bool,
    /// Extra room around the framed objects when fitting, as a fraction of
    /// their extent (0.12 = 12% margin).
    pub fit_padding: f32,
    /// Fit to the new frame's content whenever the active frame changes (see
    /// [`ViewCubeFrame::content`]), so picking "Building B" in the dropdown
    /// also brings Building B into view. Off by default.
    pub fit_on_frame_change: bool,
    /// With `double_click_projection`, a single click on the home button waits
    /// this long for a second click before snapping to iso, so a double-click
    /// only toggles the projection. Keep it below the picking multi-click
    /// interval (500 ms by default); `ZERO` snaps immediately (a double-click
    /// then snaps *and* toggles).
    pub double_click_delay: std::time::Duration,
}

impl Default for ViewCubeConfig {
    fn default() -> Self {
        Self {
            active: false,
            origin: UVec2::ZERO,
            panel_size: UVec2::ZERO,
            corner: CubeCorner::TopLeft,
            viewport_px: 128,
            inset_px: 12,
            drag_sensitivity: 1.0,
            double_click_projection: true,
            fit_padding: 0.12,
            fit_on_frame_change: false,
            double_click_delay: std::time::Duration::from_millis(300),
        }
    }
}

/// Text shown on each axis arrow / local-axes letter. Defaults to `X` / `Y` /
/// `Z`; a host with domain-specific axes overrides them (e.g. a spectrogram
/// editor sets `Time` / `Mag` / `Freq`).
///
/// Each field names the **positive** end of that axis. Keep them short — they
/// sit on small arrow tips.
#[derive(Debug, Clone)]
pub struct ViewCubeLabels {
    pub x: String,
    pub y: String,
    pub z: String,
}

impl Default for ViewCubeLabels {
    fn default() -> Self {
        Self {
            x: "X".into(),
            y: "Y".into(),
            z: "Z".into(),
        }
    }
}

/// Face captions for the [`ViewCubeStyle::Cad`] cube.
#[derive(Debug, Clone)]
pub struct ViewCubeFaceLabels {
    pub top: String,
    pub bottom: String,
    pub front: String,
    pub back: String,
    pub left: String,
    pub right: String,
}

impl Default for ViewCubeFaceLabels {
    fn default() -> Self {
        Self {
            top: "TOP".into(),
            bottom: "BASE".into(),
            front: "FRONT".into(),
            back: "BACK".into(),
            left: "LEFT".into(),
            right: "RIGHT".into(),
        }
    }
}

/// Look of the gizmo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewCubeStyle {
    /// AutoCAD-style: a solid cube with captioned faces. Faces, edges and
    /// corners (26 regions) are each clickable, plus a home button for the iso
    /// view.
    #[default]
    Cad,
    /// Blender-style: three open axis arrows (see [`ViewCubeLabels`]).
    Axes,
}

/// Options fixed when the plugin is built. Unlike [`ViewCubeConfig`] they are
/// not resources: the cube is spawned once from them, so changing them later
/// would do nothing.
#[derive(Debug, Clone)]
pub struct ViewCubeSettings {
    /// Cube look.
    pub style: ViewCubeStyle,
    /// Where the active frame's X / Y / Z axes are drawn
    /// ([`ViewCubeStyle::Cad`] only).
    pub axes: AxesPlacement,
    /// Render layer the cube and its camera live on. Must be distinct from the
    /// host scene's layers.
    pub render_layer: usize,
    /// Path (relative to the asset root) of the font used for all cube text, or
    /// `None` for Bevy's built-in default font (no asset files needed). Both
    /// TrueType and OpenType work.
    pub font: Option<String>,
    /// Axis letters.
    pub axis_labels: ViewCubeLabels,
    /// Face captions.
    pub face_labels: ViewCubeFaceLabels,
}

impl Default for ViewCubeSettings {
    fn default() -> Self {
        Self {
            style: ViewCubeStyle::Cad,
            axes: AxesPlacement::ViewportCorner(CubeCorner::BottomLeft),
            render_layer: 31,
            font: None,
            axis_labels: ViewCubeLabels::default(),
            face_labels: ViewCubeFaceLabels::default(),
        }
    }
}

/// The settings, as the cube's systems read them.
#[derive(Resource, Debug, Clone)]
pub(crate) struct CubeSettings(pub ViewCubeSettings);

/// Emitted to snap the target camera to a [`ViewPreset`]. Written by face
/// clicks; the host may also write it (e.g. from a hotkey). Becomes a
/// [`CameraAction::LookTo`] request.
#[derive(Message, Debug, Clone, Copy)]
pub struct SnapView(pub ViewPreset);

/// Frame the [`FitTarget`] entities, else the active frame's
/// [`content`](ViewCubeFrame::content), else the whole scene, without changing
/// the view direction. Written by the fit button; hosts can
/// write it too (e.g. on `F`). Consumed by `fit_view`.
#[derive(Message, Debug, Clone, Copy)]
pub struct FitView;

/// What a fit framed. See [`FitStarted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitSource {
    /// The [`FitTarget`]-tagged entities.
    Selection,
    /// The content of the active frame, by index into [`ViewCubeFrames::frames`].
    Frame(usize),
    /// Everything with a bounding box.
    Scene,
}

/// Emitted when a fit starts, saying what it is framing. Read it to show an OSD
/// ("Fit: Building B").
#[derive(Message, Debug, Clone, Copy)]
pub struct FitStarted {
    /// The camera being moved (the one carrying [`ViewCubeTarget`]).
    pub camera: Entity,
    pub source: FitSource,
}

/// Roll the last snapped view to the opposite up axis. Written on the second
/// click of a double-click on the cube; the host may write it too. Consumed by
/// `flip_view_up`.
#[derive(Message, Debug, Clone, Copy)]
pub struct FlipViewUp;

/// One named coordinate frame the cube can be aligned to.
#[derive(Debug, Clone)]
pub struct ViewCubeFrame {
    /// Shown in the frame dropdown above the cube.
    pub name: String,
    /// Orientation of the frame's axes in world space. The cube's faces,
    /// edges and corners — and the views they snap to — are expressed in it.
    pub rotation: Quat,
    /// The entity (and its descendants) this frame is about, e.g. the building
    /// a "Building B" frame belongs to. While the frame is active, fitting
    /// frames this content instead of the whole scene (a [`FitTarget`]
    /// selection still wins). `None` for frames with no content of their own,
    /// like "World".
    pub content: Option<Entity>,
}

impl ViewCubeFrame {
    pub fn new(name: impl Into<String>, rotation: Quat) -> Self {
        Self {
            name: name.into(),
            rotation,
            content: None,
        }
    }

    /// Sets the entity (and descendants) this frame's fit frames.
    pub fn with_content(mut self, content: Entity) -> Self {
        self.content = Some(content);
        self
    }
}

/// Optional coordinate-frame selector (Rhino-style). When `frames` is
/// non-empty, a dropdown above the cube ([`ViewCubeStyle::Cad`] with the `ui`
/// feature) lists the frames and shows the active one's name, the cube aligns to that frame's axes, and snaps
/// resolve in it: with a "Building" frame rotated 30°, clicking FRONT looks
/// straight at the *building's* front.
///
/// The host owns the list and may edit it at any time — e.g. keep a
/// "Selected plane" frame in sync with the selection. Leave it empty (the
/// default) for a plain world-aligned cube without the dropdown.
#[derive(Resource, Debug, Clone)]
pub struct ViewCubeFrames {
    pub frames: Vec<ViewCubeFrame>,
    /// Index into `frames` of the frame the cube is aligned to.
    pub active: usize,
}

impl Default for ViewCubeFrames {
    fn default() -> Self {
        Self {
            frames: Vec::new(),
            active: 0,
        }
    }
}

impl ViewCubeFrames {
    /// Rotation of the active frame (identity when there are no frames).
    pub fn rotation(&self) -> Quat {
        self.frames
            .get(self.active)
            .map_or(Quat::IDENTITY, |f| f.rotation)
    }
}

/// Request to make frame `index` of [`ViewCubeFrames`] active. Written by the
/// built-in dropdown; a host with its own UI writes it too. Out-of-range
/// indices are ignored. Consumed by `apply_set_frame`.
#[derive(Message, Debug, Clone, Copy)]
pub struct SetFrame(pub usize);

/// Emitted when [`ViewCubeFrames::active`] changes, whether from the dropdown or
/// the host.
#[derive(Message, Debug, Clone)]
pub struct FrameChanged {
    pub index: usize,
    pub name: String,
}

/// Camera projection kind, as reported by [`ProjectionChanged`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewProjection {
    Perspective,
    Orthographic,
}

/// Request to flip the target camera between perspective and orthographic.
/// Written on home-button double-click (see
/// [`ViewCubeConfig::double_click_projection`]); the host may also write it
/// (e.g. from a hotkey). Becomes a [`CameraAction::SetProjection`] request.
#[derive(Message, Debug, Clone, Copy)]
pub struct ToggleProjection;

/// Emitted when the cube starts switching the target camera's projection, with
/// the projection it is switching *to*. Read this to show an OSD, update a
/// toolbar toggle, etc.
#[derive(Message, Debug, Clone, Copy)]
pub struct ProjectionChanged {
    /// The camera being switched (the one carrying [`ViewCubeTarget`]).
    pub camera: Entity,
    pub projection: ViewProjection,
}

/// The view-cube plugin. Carries the initial [`ViewCubeConfig`], the
/// build-time [`ViewCubeSettings`] and the optional [`ViewCubeFrames`]. Add a
/// driver plugin ([`driver`]) too, or nothing will move the camera.
#[derive(Default)]
pub struct ViewCubePlugin {
    pub config: ViewCubeConfig,
    pub settings: ViewCubeSettings,
    pub frames: ViewCubeFrames,
}

impl Plugin for ViewCubePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(text3d::CaptionPlugin);
        app.insert_resource(self.config)
            .insert_resource(CubeSettings(self.settings.clone()))
            .insert_resource(self.frames.clone())
            .init_resource::<systems::CubeDragState>()
            .init_resource::<systems::LastSnap>()
            .add_message::<CameraRequest>()
            .add_message::<SnapView>()
            .add_message::<FlipViewUp>()
            .add_message::<FitView>()
            .add_message::<FitStarted>()
            .add_message::<ToggleProjection>()
            .add_message::<ProjectionChanged>()
            .add_message::<SetFrame>()
            .add_message::<FrameChanged>()
            .add_systems(Startup, cube::setup_view_cube)
            .add_systems(
                Update,
                (
                    systems::snap_view,
                    systems::flip_view_up.after(systems::snap_view),
                    systems::toggle_projection,
                    fit::fit_on_frame_change,
                    fit::fit_view.after(fit::fit_on_frame_change),
                    systems::apply_set_frame,
                    systems::sync_frames,
                    systems::mirror_camera_orientation,
                    systems::billboard_axis_labels.after(systems::mirror_camera_orientation),
                    systems::sync_viewport_axes.after(systems::mirror_camera_orientation),
                    systems::sync_cube_camera_active,
                )
                    .in_set(ViewCubeSet),
            )
            .add_systems(PostUpdate, systems::sync_cube_viewport);

        #[cfg(feature = "ui")]
        ui::build(app);
    }
}
