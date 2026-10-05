//! The seam between the cube and whatever moves the camera.
//!
//! The cube decides *what* should happen — look along this direction, orbit by
//! this much, frame these bounds — and says so with a [`CameraRequest`]. It
//! never calls a camera controller. A **driver** reads the requests and moves
//! the [`ViewCubeTarget`](crate::ViewCubeTarget) camera:
//!
//! - [`BasicDriverPlugin`]: orbits a plain `Camera3d` itself; no extra
//!   dependency.
//! - [`EditorCamDriverPlugin`] (feature `editor_cam`): adapts
//!   [`bevy_editor_cam`]'s `EditorCam`.
//! - Your own: read [`CameraRequest`] in a system ordered
//!   `.after(ViewCubeSet)`. Add [`TweenPlugin`] to get the animated actions
//!   ([`CameraAction::LookTo`], [`CameraAction::Fit`]) for free, and remove
//!   [`Tween`] from the camera when the user takes over; or just set the
//!   transform yourself.
//!
//! A driver also keeps [`ViewCubeFocus`] up to date: the distance from the
//! camera to the point it orbits.

use bevy::prelude::*;

use crate::ViewProjection;

mod basic;
#[cfg(feature = "editor_cam")]
mod editor_cam;
mod tween;

pub use basic::BasicDriverPlugin;
#[cfg(feature = "editor_cam")]
pub use editor_cam::EditorCamDriverPlugin;
pub use tween::{Tween, TweenPlugin};

/// A request from the cube to the camera driver.
#[derive(Message, Debug, Clone)]
pub struct CameraRequest {
    /// The camera to move: the one carrying
    /// [`ViewCubeTarget`](crate::ViewCubeTarget).
    pub camera: Entity,
    pub action: CameraAction,
}

/// What the cube wants the camera to do.
#[derive(Debug, Clone)]
pub enum CameraAction {
    /// Rotate about the focus point (at [`ViewCubeFocus`] in front of the
    /// camera) until looking along `facing` with `up` up on screen. Animated.
    LookTo { facing: Dir3, up: Dir3 },
    /// A drag on the cube starts orbiting.
    OrbitBegin,
    /// Orbit by this pointer movement (logical px, already scaled by
    /// `ViewCubeConfig::drag_sensitivity`), as if dragging the scene.
    OrbitDelta(Vec2),
    /// The drag ended; let any momentum play out.
    OrbitEnd,
    /// Switch to this projection, keeping the framing of the focus point.
    SetProjection(ViewProjection),
    /// Move to `position` to frame some bounds, keeping the view direction.
    /// `ortho_scale` is the new orthographic scale (`None` for perspective);
    /// `focus_distance` is the distance from `position` to the framed center,
    /// which becomes the new orbit pivot. Animated.
    Fit {
        position: Vec3,
        ortho_scale: Option<f32>,
        focus_distance: f32,
    },
    /// The orientation of the active coordinate frame changed. Drivers that
    /// constrain orbiting to an up axis should use `frame_rotation * their
    /// world up`, so orbiting after a snap in a rotated frame keeps the
    /// frame's up. Sent once at startup (identity) and on every change.
    AlignUp { frame_rotation: Quat },
}

/// Distance from the target camera to the point it orbits and zooms about
/// (view-space depth of the anchor). Drivers keep it current; the cube uses it
/// to frame a lone point without changing the zoom, and the shared tween uses
/// it as the orbit radius for [`CameraAction::LookTo`].
#[derive(Component, Debug, Clone, Copy)]
pub struct ViewCubeFocus(pub f32);

impl Default for ViewCubeFocus {
    fn default() -> Self {
        Self(10.0)
    }
}
