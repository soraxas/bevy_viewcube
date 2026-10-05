//! Runtime systems: mirror the target camera, billboard the letters, turn
//! clicks and drags into camera requests, pin the viewport, and gate activity
//! on [`ViewCubeConfig`].
//!
//! Nothing here touches a camera controller: the cube reads the target's
//! `Transform` / `Projection` and writes [`CameraRequest`]s for a driver to
//! carry out (see [`crate::driver`]).

use bevy::prelude::*;

use crate::cube::{CubeAxisLabel, CubeCamera, CubeFace, CubeHover, CubePivot, CubeViewportAxes};
use crate::driver::{CameraAction, CameraRequest};
use crate::{
    CubeCorner, FlipViewUp, FrameChanged, ProjectionChanged, SetFrame, SnapView, ToggleProjection,
    ViewCubeConfig, ViewCubeFrames, ViewCubeTarget, ViewProjection,
};

/// The last view the cube snapped to, so [`FlipViewUp`] can roll *that* view
/// even while its animation is still running.
#[derive(Resource, Debug, Default)]
pub(crate) struct LastSnap {
    /// World-space facing direction and screen-up direction of the snap.
    view: Option<(Dir3, Dir3)>,
}

/// Pick the screen-up direction for a view facing `facing` (world space).
///
/// Candidates are the active frame's six axes, flattened into the plane
/// perpendicular to the view. The one closest to the camera's current up wins
/// (the smallest roll). If the current up is orthogonal to every candidate
/// (e.g. going from TOP to FRONT) there's no closest one, so fall back to the
/// frame's own up: its +Y, or for TOP / BOTTOM the frame's -Z / +Z.
fn choose_up(facing: Vec3, frame: Quat, current_up: Vec3) -> Option<Vec3> {
    let candidates: Vec<Vec3> = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z]
        .into_iter()
        .filter_map(|axis| {
            let a = frame * axis;
            let flat = a - facing * a.dot(facing);
            (flat.length() > 0.05).then(|| flat.normalize())
        })
        .collect();
    let (best, dot) = candidates
        .iter()
        .map(|c| (*c, c.dot(current_up)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    if dot > 1e-3 {
        return Some(best);
    }
    let frame_up = frame * Vec3::Y;
    let flat = frame_up - facing * frame_up.dot(facing);
    if flat.length() > 0.05 {
        return Some(flat.normalize());
    }
    // Looking along the frame's up axis: TOP puts the frame's -Z (north) up.
    let z = frame * if facing.dot(frame_up) < 0.0 { Vec3::NEG_Z } else { Vec3::Z };
    Some((z - facing * z.dot(facing)).normalize_or_zero()).filter(|v| *v != Vec3::ZERO)
}

/// Turn [`SnapView`] requests into [`CameraAction::LookTo`], resolving the
/// facing and up direction in the active frame.
pub(crate) fn snap_view(
    mut requests: MessageReader<SnapView>,
    mut out: MessageWriter<CameraRequest>,
    mut last: ResMut<LastSnap>,
    frames: Res<ViewCubeFrames>,
    cam_q: Query<(Entity, &Transform), With<ViewCubeTarget>>,
) {
    let Ok((entity, transform)) = cam_q.single() else {
        requests.clear();
        return;
    };
    let frame = frames.rotation();
    for SnapView(preset) in requests.read().copied() {
        // Presets are expressed in the active frame; rotate into world space.
        let Ok(facing) = Dir3::new(frame * preset.facing()) else {
            continue;
        };
        let Some(up) = choose_up(*facing, frame, *transform.up()).and_then(|u| Dir3::new(u).ok())
        else {
            continue;
        };
        last.view = Some((facing, up));
        out.write(CameraRequest {
            camera: entity,
            action: CameraAction::LookTo { facing, up },
        });
    }
}

/// Roll the last snapped view to the opposite up axis (the furthest candidate
/// from the one the snap chose).
pub(crate) fn flip_view_up(
    mut requests: MessageReader<FlipViewUp>,
    mut out: MessageWriter<CameraRequest>,
    mut last: ResMut<LastSnap>,
    cam_q: Query<Entity, With<ViewCubeTarget>>,
) {
    let Ok(entity) = cam_q.single() else {
        requests.clear();
        return;
    };
    for _ in requests.read() {
        let Some((facing, up)) = last.view else {
            continue;
        };
        let up = -up;
        last.view = Some((facing, up));
        out.write(CameraRequest {
            camera: entity,
            action: CameraAction::LookTo { facing, up },
        });
    }
}

/// Turn [`ToggleProjection`] requests into a [`CameraAction::SetProjection`],
/// and announce it with [`ProjectionChanged`].
pub(crate) fn toggle_projection(
    mut requests: MessageReader<ToggleProjection>,
    mut out: MessageWriter<CameraRequest>,
    mut changed: MessageWriter<ProjectionChanged>,
    cam_q: Query<(Entity, &Projection), With<ViewCubeTarget>>,
) {
    let Ok((camera, projection)) = cam_q.single() else {
        requests.clear();
        return;
    };
    // Several requests in a frame cancel in pairs; apply the net result once.
    if requests.read().count() % 2 == 0 {
        return;
    }
    let to = match projection {
        Projection::Perspective(_) => ViewProjection::Orthographic,
        Projection::Orthographic(_) => ViewProjection::Perspective,
        Projection::Custom(_) => return,
    };
    out.write(CameraRequest {
        camera,
        action: CameraAction::SetProjection(to),
    });
    changed.write(ProjectionChanged {
        camera,
        projection: to,
    });
}

/// Rotate the cube pivot to mirror the target camera's orientation, so the
/// face pointing at the cube camera matches the current view.
pub(crate) fn mirror_camera_orientation(
    target: Query<&Transform, (With<ViewCubeTarget>, Without<CubePivot>)>,
    frames: Res<ViewCubeFrames>,
    mut pivot: Query<&mut Transform, With<CubePivot>>,
) {
    let Ok(scene) = target.single() else { return };
    let Ok(mut pivot) = pivot.single_mut() else {
        return;
    };
    // The cube shows the orientation the target camera views its (active)
    // frame from: the frame's axes seen through the inverse camera rotation.
    pivot.rotation = scene.rotation.inverse() * frames.rotation();
}

/// Keep each axis letter facing the (fixed) cube camera and held just off its
/// anchor toward the viewer. The letters are children of the pivot, which
/// rotates to mirror the target camera — that would carry the letters edge-on
/// and swing them around. Setting each label's local rotation to the pivot's
/// inverse cancels that (world rotation stays identity, front-facing the +Z
/// camera), and the translation is its stored `base` plus a camera-facing
/// nudge (also expressed in pivot-local space) so it sits in front of the
/// arrow tip at every orientation.
pub(crate) fn billboard_axis_labels(
    pivot: Query<&Transform, With<CubePivot>>,
    mut labels: Query<(&mut Transform, &CubeAxisLabel), Without<CubePivot>>,
) {
    let Ok(pivot) = pivot.single() else { return };
    let inv = pivot.rotation.inverse();
    let cam_nudge = inv * (Vec3::Z * crate::cube::LABEL_CAM_OFFSET);
    for (mut label, anchor) in &mut labels {
        if label.rotation != inv {
            label.rotation = inv;
        }
        let want = anchor.base + cam_nudge;
        if label.translation != want {
            label.translation = want;
        }
    }
}

/// Activate the cube camera only while [`ViewCubeConfig::active`].
pub(crate) fn sync_cube_camera_active(
    config: Res<ViewCubeConfig>,
    mut cam_q: Query<&mut Camera, With<CubeCamera>>,
    mut pivot_q: Query<&mut Visibility, With<CubePivot>>,
) {
    let show = config.active;
    for mut cam in &mut cam_q {
        if cam.is_active != show {
            cam.is_active = show;
        }
    }
    for mut vis in &mut pivot_q {
        let want = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
}

/// Physical-pixel rect `(position, size)` of the cube viewport inside the host
/// rect, DPI-scaled; `None` until the host has supplied a rect.
pub(crate) fn cube_viewport_rect(config: &ViewCubeConfig, scale: f32) -> Option<(UVec2, UVec2)> {
    if config.panel_size.x < 2 || config.panel_size.y < 2 {
        return None;
    }
    let side = ((config.viewport_px as f32) * scale).round().max(1.0) as u32;
    let inset = (config.inset_px as f32 * scale).round() as u32;

    let max_x = config.origin.x + config.panel_size.x;
    let max_y = config.origin.y + config.panel_size.y;
    let (x, y) = match config.corner {
        CubeCorner::TopLeft => (config.origin.x + inset, config.origin.y + inset),
        CubeCorner::TopRight => (max_x.saturating_sub(side + inset), config.origin.y + inset),
        CubeCorner::BottomLeft => (config.origin.x + inset, max_y.saturating_sub(side + inset)),
        CubeCorner::BottomRight => (
            max_x.saturating_sub(side + inset),
            max_y.saturating_sub(side + inset),
        ),
    };
    Some((UVec2::new(x, y), UVec2::splat(side)))
}

/// Pin the cube camera's viewport to a fixed square in the configured corner
/// of the host rect (physical px), DPI-scaled.
pub(crate) fn sync_cube_viewport(
    config: Res<ViewCubeConfig>,
    windows: Query<&Window>,
    mut cam_q: Query<&mut Camera, With<CubeCamera>>,
) {
    let scale = windows
        .iter()
        .next()
        .map(|w| w.scale_factor())
        .unwrap_or(1.0);
    let Some((position, size)) = cube_viewport_rect(&config, scale) else {
        return;
    };
    let vp = bevy::camera::Viewport {
        physical_position: position,
        physical_size: size,
        depth: 0.0..1.0,
    };
    for mut cam in &mut cam_q {
        let needs = match &cam.viewport {
            Some(e) => {
                e.physical_position != vp.physical_position || e.physical_size != vp.physical_size
            }
            None => true,
        };
        if needs {
            cam.viewport = Some(vp.clone());
        }
    }
}

/// Tracks whether the current press on the cube turned into a drag, so the
/// click that follows the release doesn't also snap the view.
#[derive(Resource, Debug, Default)]
pub(crate) struct CubeDragState {
    dragged: bool,
}

/// Pointer travel (logical px) before a press counts as a drag, not a click.
const DRAG_THRESHOLD: f32 = 4.0;

/// Reset the drag flag at the start of every press on a cube region.
pub(crate) fn on_cube_press(press: On<Pointer<Press>>, mut state: ResMut<CubeDragState>) {
    if press.button == PointerButton::Primary {
        state.dragged = false;
    }
}

/// Begin an orbit when a cube drag starts. The driver decides how.
pub(crate) fn on_cube_drag_start(
    start: On<Pointer<DragStart>>,
    mut out: MessageWriter<CameraRequest>,
    cam_q: Query<Entity, With<ViewCubeTarget>>,
) {
    // Only the primary button drives the cube; right / middle belong to the
    // host's own camera controls, which handle them over the whole viewport.
    if start.button != PointerButton::Primary {
        return;
    }
    if let Ok(camera) = cam_q.single() {
        out.write(CameraRequest {
            camera,
            action: CameraAction::OrbitBegin,
        });
    }
}

/// Feed cube drag deltas to the driver as orbit input. Dragging the cube turns
/// it with the pointer, like grabbing the scene.
pub(crate) fn on_cube_drag(
    drag: On<Pointer<Drag>>,
    config: Res<ViewCubeConfig>,
    mut state: ResMut<CubeDragState>,
    mut out: MessageWriter<CameraRequest>,
    cam_q: Query<Entity, With<ViewCubeTarget>>,
) {
    if drag.button != PointerButton::Primary {
        return;
    }
    if drag.distance.length() > DRAG_THRESHOLD {
        state.dragged = true;
    }
    if let Ok(camera) = cam_q.single() {
        out.write(CameraRequest {
            camera,
            action: CameraAction::OrbitDelta(drag.delta * config.drag_sensitivity),
        });
    }
}

/// End the orbit when the cube drag is released.
pub(crate) fn on_cube_drag_end(
    end: On<Pointer<DragEnd>>,
    mut out: MessageWriter<CameraRequest>,
    cam_q: Query<Entity, With<ViewCubeTarget>>,
) {
    if end.button != PointerButton::Primary {
        return;
    }
    if let Ok(camera) = cam_q.single() {
        out.write(CameraRequest {
            camera,
            action: CameraAction::OrbitEnd,
        });
    }
}

/// On click of a cube region, emit the matching [`SnapView`]; on the second
/// click of a double-click, [`FlipViewUp`] (roll the view to the other up
/// axis). Attached per-region via `.observe()` at spawn. Ignored when the press
/// was a drag.
pub(crate) fn on_cube_face_click(
    click: On<Pointer<Click>>,
    state: Res<CubeDragState>,
    faces: Query<&CubeFace>,
    mut snap: MessageWriter<SnapView>,
    mut flip: MessageWriter<FlipViewUp>,
) {
    if state.dragged || click.button != PointerButton::Primary {
        return;
    }
    match click.count {
        1 => {
            if let Ok(face) = faces.get(click.entity) {
                snap.write(SnapView(face.0));
            }
        }
        2 => {
            flip.write(FlipViewUp);
        }
        _ => {}
    }
}

/// Swap a CAD cube region to its hover or resting material.
fn set_hover(
    entity: Entity,
    hovered: bool,
    regions: &mut Query<(&CubeHover, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    if let Ok((hover, mut mat)) = regions.get_mut(entity) {
        mat.0 = if hovered {
            hover.hover.clone()
        } else {
            hover.base.clone()
        };
    }
}

/// Highlight a CAD cube region while hovered.
pub(crate) fn on_cube_hover_start(
    over: On<Pointer<Over>>,
    mut regions: Query<(&CubeHover, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    set_hover(over.entity, true, &mut regions);
}

/// Restore a CAD cube region's resting material when the pointer leaves.
pub(crate) fn on_cube_hover_end(
    out: On<Pointer<Out>>,
    mut regions: Query<(&CubeHover, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    set_hover(out.entity, false, &mut regions);
}

/// Apply [`SetFrame`] requests (from the built-in dropdown or the host).
pub(crate) fn apply_set_frame(mut requests: MessageReader<SetFrame>, mut frames: ResMut<ViewCubeFrames>) {
    for SetFrame(index) in requests.read().copied() {
        if index < frames.frames.len() && frames.active != index {
            frames.active = index;
        }
    }
}

/// Keep the cube in step with [`ViewCubeFrames`]: announce changes of the
/// active frame ([`FrameChanged`], and [`CameraAction::AlignUp`] to the
/// driver) and make room at the top of the cube
/// viewport for the frame dropdown when there are frames to pick from.
pub(crate) fn sync_frames(
    frames: Res<ViewCubeFrames>,
    mut changed: MessageWriter<FrameChanged>,
    mut out: MessageWriter<CameraRequest>,
    mut last_active: Local<Option<usize>>,
    mut last_rotation: Local<Option<Quat>>,
    target: Query<Entity, With<ViewCubeTarget>>,
    mut cam: Query<&mut Projection, With<CubeCamera>>,
) {
    // Tell the driver which way "up" is whenever the active frame's
    // orientation changes (and once at startup).
    let rotation = frames.rotation();
    if *last_rotation != Some(rotation) {
        if let Ok(camera) = target.single() {
            out.write(CameraRequest {
                camera,
                action: CameraAction::AlignUp {
                    frame_rotation: rotation,
                },
            });
            *last_rotation = Some(rotation);
        }
    }

    // Announce active-frame changes (dropdown or host), skipping the first frame.
    let active = frames.frames.get(frames.active).map(|_| frames.active);
    if let (Some(prev), Some(index)) = (*last_active, active) {
        if prev != index {
            changed.write(FrameChanged {
                index,
                name: frames.frames[index].name.clone(),
            });
        }
    }
    *last_active = active;

    if !frames.is_changed() {
        return;
    }
    // Only the built-in UI draws a dropdown over the viewport; without it
    // there's nothing to leave room for.
    let show = cfg!(feature = "ui") && !frames.frames.is_empty();
    let height = if show {
        crate::cad::SELECTOR_HEIGHT
    } else {
        crate::cad::CUBE_HEIGHT
    };
    for mut proj in &mut cam {
        if let Projection::Orthographic(o) = &mut *proj {
            o.scaling_mode = bevy::camera::ScalingMode::FixedVertical {
                viewport_height: height,
            };
        }
    }
}

/// Inset (cube-space units) of the viewport-corner triad's center from the
/// viewport edges: room for the arrows and letters in any orientation.
const VIEWPORT_AXES_INSET: f32 = 0.62;

/// Pin the viewport-corner axes triad to its corner and turn it with the cube.
///
/// The triad is a child of the (fixed) cube camera, so its local rotation is
/// the pivot's world rotation expressed in camera space.
pub(crate) fn sync_viewport_axes(
    pivot: Query<&Transform, (With<CubePivot>, Without<CubeViewportAxes>, Without<CubeCamera>)>,
    cam: Query<(&Transform, &Projection), (With<CubeCamera>, Without<CubeViewportAxes>)>,
    mut axes: Query<(&CubeViewportAxes, &mut Transform), Without<CubeCamera>>,
) {
    let (Ok(pivot), Ok((cam_t, projection))) = (pivot.single(), cam.single()) else {
        return;
    };
    let Projection::Orthographic(ortho) = projection else {
        return;
    };
    let bevy::camera::ScalingMode::FixedVertical { viewport_height } = ortho.scaling_mode else {
        return;
    };
    let half = viewport_height * 0.5 - VIEWPORT_AXES_INSET;
    let rotation = cam_t.rotation.inverse() * pivot.rotation;
    for (CubeViewportAxes(corner), mut t) in &mut axes {
        let (sx, sy) = match corner {
            CubeCorner::TopLeft => (-1.0, 1.0),
            CubeCorner::TopRight => (1.0, 1.0),
            CubeCorner::BottomLeft => (-1.0, -1.0),
            CubeCorner::BottomRight => (1.0, -1.0),
        };
        // z = -2: in front of the cube (which spans ±1.5 about the camera's
        // focus at distance 4), so the triad never z-fights with it.
        t.translation = Vec3::new(sx * half, sy * half, -2.0);
        t.rotation = rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn side_view_keeps_frame_up() {
        let up = choose_up(Vec3::NEG_Z, Quat::IDENTITY, Vec3::Y).unwrap();
        assert!(close(up, Vec3::Y), "{up:?}");
    }

    #[test]
    fn side_view_in_rotated_frame_aligns_to_frame_up() {
        // A frame tilted off the world axes: the up must be the frame's +Y,
        // not the world's.
        let frame = Quat::from_euler(EulerRot::YXZ, 0.5, 0.3, 0.0);
        let facing = frame * Vec3::NEG_Z;
        let current = frame * Vec3::Y + Vec3::new(0.02, 0.0, 0.01);
        let up = choose_up(facing, frame, current.normalize()).unwrap();
        assert!(close(up, frame * Vec3::Y), "{up:?}");
    }

    #[test]
    fn top_view_picks_nearest_horizontal_axis() {
        // Camera tipped so its up leans toward -Z: TOP puts -Z up.
        let current = Vec3::new(0.0, 0.8, -0.6).normalize();
        let up = choose_up(Vec3::NEG_Y, Quat::IDENTITY, current).unwrap();
        assert!(close(up, Vec3::NEG_Z), "{up:?}");
        // Leaning toward +X instead.
        let current = Vec3::new(0.7, 0.7, 0.1).normalize();
        let up = choose_up(Vec3::NEG_Y, Quat::IDENTITY, current).unwrap();
        assert!(close(up, Vec3::X), "{up:?}");
    }

    #[test]
    fn orthogonal_current_up_falls_back_to_frame_default() {
        // FRONT -> TOP: current up (+Y) is orthogonal to every candidate.
        let up = choose_up(Vec3::NEG_Y, Quat::IDENTITY, Vec3::Y).unwrap();
        assert!(close(up, Vec3::NEG_Z), "{up:?}");
        // TOP -> FRONT: current up (-Z) is orthogonal to the candidates; use +Y.
        let up = choose_up(Vec3::NEG_Z, Quat::IDENTITY, Vec3::NEG_Z).unwrap();
        assert!(close(up, Vec3::Y), "{up:?}");
        // BOTTOM default is +Z.
        let up = choose_up(Vec3::Y, Quat::IDENTITY, Vec3::Y).unwrap();
        assert!(close(up, Vec3::Z), "{up:?}");
    }

    #[test]
    fn iso_view_keeps_frame_up() {
        let facing = Vec3::new(-0.6, -0.55, -1.0).normalize();
        let up = choose_up(facing, Quat::IDENTITY, Vec3::Y).unwrap();
        // Closest candidate is the +Y axis flattened against the view.
        assert!(up.dot(Vec3::Y) > 0.8, "{up:?}");
    }
}
