//! Adapter for [`bevy_editor_cam`]: carries out the cube's [`CameraRequest`]s on
//! an `EditorCam`. This is the **only** place the crate touches that
//! controller, so what it relies on is listed here:
//!
//! - `EditorCam::{start_orbit, send_screenspace_input, end_move}` and
//!   `current_motion` — public API, the documented way to drive it.
//! - `DollyZoomTrigger` — its public projection-switch message.
//! - `last_anchor_depth` — read to publish [`ViewCubeFocus`], written when a
//!   fit moves the pivot.
//! - `orbit_constraint` — rewritten (opt out with
//!   [`EditorCamDriverPlugin::follow_frame_up`]) so the fixed up follows the
//!   active frame.
//! - `CameraPointerMap` — the stock mouse input hooks the pointer in here; we
//!   unhook it while the cube is dragged, otherwise left-drag also pans.
//!
//! Look-to and fit animations are the crate's own ([`TweenPlugin`]), not
//! `bevy_editor_cam`'s `LookTo`, so there's no private state to reset when the
//! user grabs the camera mid-animation — the tween is simply dropped.

use bevy::prelude::*;
use bevy_editor_cam::extensions::dolly_zoom::{DollyZoomPlugin, DollyZoomTrigger};
use bevy_editor_cam::input::CameraPointerMap;
use bevy_editor_cam::prelude::{motion::CurrentMotion, EditorCam, OrbitConstraint};

use super::{CameraAction, CameraRequest, Tween, TweenPlugin, ViewCubeFocus};
use crate::{ViewCubeSet, ViewProjection};

/// Add after [`ViewCubePlugin`](crate::ViewCubePlugin); see the module source
/// for details.
pub struct EditorCamDriverPlugin {
    /// Rotate the camera's `OrbitConstraint::Fixed` up axis with the active
    /// coordinate frame, so orbiting after a snap in a rotated frame doesn't
    /// re-roll the view back to the world's up. The host's own up is restored
    /// when the frame returns to identity. A no-op for `OrbitConstraint::Free`.
    pub follow_frame_up: bool,
}

impl Default for EditorCamDriverPlugin {
    fn default() -> Self {
        Self {
            follow_frame_up: true,
        }
    }
}

#[derive(Resource)]
struct EditorCamDriver {
    follow_frame_up: bool,
}

impl Plugin for EditorCamDriverPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<TweenPlugin>() {
            app.add_plugins(TweenPlugin);
        }
        if !app.is_plugin_added::<DollyZoomPlugin>() {
            app.add_plugins(DollyZoomPlugin);
        }
        app.insert_resource(EditorCamDriver {
            follow_frame_up: self.follow_frame_up,
        })
        .add_systems(
            Update,
            (handle_requests, sync_focus, cancel_tween_on_user_input)
                .chain()
                .after(ViewCubeSet),
        );
    }
}

/// Publish the editor cam's anchor depth as the cube's focus distance.
fn sync_focus(mut cams: Query<(&EditorCam, &mut ViewCubeFocus)>) {
    for (editor, mut focus) in &mut cams {
        let depth = editor.last_anchor_depth().abs() as f32;
        if (focus.0 - depth).abs() > f32::EPSILON {
            focus.0 = depth;
        }
    }
}

/// Drop a running animation as soon as the user takes the camera — the editor
/// cam doesn't know about our tween and would otherwise fight it.
fn cancel_tween_on_user_input(
    mut commands: Commands,
    cams: Query<(Entity, &EditorCam), With<Tween>>,
) {
    for (entity, editor) in &cams {
        if editor.current_motion.is_user_controlled() {
            commands.entity(entity).remove::<Tween>();
        }
    }
}

/// Take over the camera for an orbit driven by the cube.
///
/// The stock input maps left-drag to *pan*, and the cube sits inside the host
/// camera's viewport, so the same press also starts a pan and hooks the mouse
/// into [`CameraPointerMap`], which keeps feeding pointer motion to the
/// camera. Force the orbit (the pan, or momentum from a previous drag, may have
/// replaced it) and unhook the stock feed so deltas aren't applied twice.
fn claim_orbit(cam: &mut EditorCam, pointer_map: Option<&mut CameraPointerMap>) {
    if let Some(map) = pointer_map {
        map.remove(&bevy::picking::pointer::PointerId::Mouse);
    }
    let orbiting = matches!(
        cam.current_motion,
        CurrentMotion::UserControlled {
            motion_inputs: bevy_editor_cam::controller::inputs::MotionInputs::OrbitZoom { .. },
            ..
        }
    );
    if !orbiting {
        cam.start_orbit(None);
    }
}

fn handle_requests(
    mut requests: MessageReader<CameraRequest>,
    driver: Res<EditorCamDriver>,
    mut pointer_map: Option<ResMut<CameraPointerMap>>,
    mut dolly: MessageWriter<DollyZoomTrigger>,
    mut cams: Query<&mut EditorCam>,
    // Last frame rotation applied to the constraint, so the host's own up can
    // be recovered even if the host changed it in between.
    mut applied: Local<Quat>,
) {
    for CameraRequest { camera, action } in requests.read() {
        let Ok(mut editor) = cams.get_mut(*camera) else {
            continue;
        };
        match action {
            CameraAction::OrbitBegin => claim_orbit(&mut editor, pointer_map.as_deref_mut()),
            CameraAction::OrbitDelta(delta) => {
                // Re-assert every event: the stock pan can (re)start after
                // OrbitBegin.
                claim_orbit(&mut editor, pointer_map.as_deref_mut());
                editor.send_screenspace_input(*delta);
            }
            CameraAction::OrbitEnd => editor.end_move(),
            CameraAction::LookTo { .. } => stop_motion(&mut editor),
            CameraAction::Fit { focus_distance, .. } => {
                stop_motion(&mut editor);
                // Orbit about what we framed.
                editor.last_anchor_depth = -(*focus_distance as f64);
            }
            CameraAction::SetProjection(to) => {
                dolly.write(DollyZoomTrigger {
                    target_projection: match to {
                        ViewProjection::Perspective => {
                            Projection::Perspective(PerspectiveProjection::default())
                        }
                        ViewProjection::Orthographic => {
                            Projection::Orthographic(OrthographicProjection::default_3d())
                        }
                    },
                    camera: *camera,
                });
            }
            CameraAction::AlignUp { frame_rotation } => {
                if !driver.follow_frame_up {
                    continue;
                }
                if let OrbitConstraint::Fixed { up, can_pass_tdc } = editor.orbit_constraint {
                    let host_up = applied.inverse() * up.as_vec3();
                    let want = (*frame_rotation * host_up).as_dvec3();
                    if want.distance(up) > 1e-5 {
                        editor.orbit_constraint = OrbitConstraint::Fixed {
                            up: want,
                            can_pass_tdc,
                        };
                    }
                    *applied = *frame_rotation;
                }
            }
        }
    }
}

/// Stop any user motion / momentum so a tween isn't fought.
fn stop_motion(editor: &mut EditorCam) {
    editor.end_move();
    editor.current_motion = CurrentMotion::Stationary;
}
