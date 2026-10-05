//! Camera animations shared by every driver: look-to (rotate about the focus
//! point) and fit (slide to a pose). They only write the target's `Transform`,
//! `Projection` and [`ViewCubeFocus`]; a driver cancels one by removing
//! [`Tween`] when the user takes the camera.

use bevy::prelude::*;
use bevy::window::RequestRedraw;

use super::{CameraAction, CameraRequest, ViewCubeFocus};
use crate::ViewCubeSet;

const LOOK_SECS: f64 = 0.4;
const FIT_SECS: f64 = 0.3;

/// The look-to / fit animation running on a camera. Remove it to cancel (do so
/// when the user takes over the camera).
#[derive(Component)]
pub struct Tween {
    start: f64,
    secs: f64,
    kind: Kind,
}

enum Kind {
    Look {
        focus: Vec3,
        dist: f32,
        from: Quat,
        to: Quat,
    },
    Fit {
        from_pos: Vec3,
        to_pos: Vec3,
        from_scale: Option<f32>,
        to_scale: Option<f32>,
    },
}

/// Runs the cube's animated [`CameraAction`]s (`LookTo`, `Fit`) on the target
/// camera's `Transform` / `Projection` / [`ViewCubeFocus`]. Added by the
/// bundled drivers; add it yourself for a custom one.
pub struct TweenPlugin;

impl Plugin for TweenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (begin_tweens, run_tweens).chain().after(ViewCubeSet),
        );
    }
}

fn begin_tweens(
    mut requests: MessageReader<CameraRequest>,
    time: Res<Time>,
    mut commands: Commands,
    mut cams: Query<(&Transform, &Projection, &mut ViewCubeFocus)>,
) {
    for CameraRequest { camera, action } in requests.read() {
        let Ok((t, projection, mut focus)) = cams.get_mut(*camera) else {
            continue;
        };
        let now = time.elapsed_secs_f64();
        match action {
            CameraAction::LookTo { facing, up } => {
                commands.entity(*camera).insert(Tween {
                    start: now,
                    secs: LOOK_SECS,
                    kind: Kind::Look {
                        focus: t.translation + *t.forward() * focus.0,
                        dist: focus.0,
                        from: t.rotation,
                        to: Transform::default().looking_to(**facing, **up).rotation,
                    },
                });
            }
            CameraAction::Fit {
                position,
                ortho_scale,
                focus_distance,
            } => {
                let from_scale = match projection {
                    Projection::Orthographic(o) => Some(o.scale),
                    _ => None,
                };
                commands.entity(*camera).insert(Tween {
                    start: now,
                    secs: FIT_SECS,
                    kind: Kind::Fit {
                        from_pos: t.translation,
                        to_pos: *position,
                        from_scale,
                        to_scale: ortho_scale.filter(|_| from_scale.is_some()),
                    },
                });
                focus.0 = *focus_distance;
            }
            _ => {}
        }
    }
}

fn run_tweens(
    time: Res<Time>,
    mut commands: Commands,
    mut redraw: MessageWriter<RequestRedraw>,
    mut cams: Query<(Entity, &Tween, &mut Transform, &mut Projection)>,
) {
    for (entity, tween, mut t, mut projection) in &mut cams {
        let linear = ((time.elapsed_secs_f64() - tween.start) / tween.secs).clamp(0.0, 1.0) as f32;
        let eased = 1.0 - (1.0 - linear).powi(3);
        match &tween.kind {
            Kind::Look {
                focus,
                dist,
                from,
                to,
            } => {
                t.rotation = from.slerp(*to, eased);
                t.translation = *focus - *t.forward() * *dist;
            }
            Kind::Fit {
                from_pos,
                to_pos,
                from_scale,
                to_scale,
            } => {
                t.translation = from_pos.lerp(*to_pos, eased);
                if let (Some(a), Some(b), Projection::Orthographic(o)) =
                    (from_scale, to_scale, &mut *projection)
                {
                    o.scale = a + (b - a) * eased;
                }
            }
        }
        redraw.write(RequestRedraw);
        if linear >= 1.0 {
            commands.entity(entity).remove::<Tween>();
        }
    }
}
