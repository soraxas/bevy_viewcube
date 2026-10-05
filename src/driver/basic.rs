//! A dependency-free driver for a plain `Camera3d`: orbits about a focus point
//! at [`ViewCubeFocus`] in front of the camera, switches projection, and runs
//! the shared look-to / fit animations.
//!
//! It only handles what the cube asks for (dragging or clicking the cube). It
//! adds no mouse controls of its own, so pair it with whatever you already use
//! to move the camera; keep [`ViewCubeFocus`] honest if that moves the pivot.

use bevy::prelude::*;

use super::{CameraAction, CameraRequest, Tween, TweenPlugin, ViewCubeFocus};
use crate::{ViewCubeSet, ViewProjection};

/// Add after [`ViewCubePlugin`](crate::ViewCubePlugin); see the module source
/// for details.
pub struct BasicDriverPlugin {
    /// The world's up axis, which orbiting yaws about and never rolls away
    /// from.
    pub up: Vec3,
    /// Radians of orbit per logical pixel of cube drag.
    pub orbit_speed: f32,
}

impl Default for BasicDriverPlugin {
    fn default() -> Self {
        Self {
            up: Vec3::Y,
            orbit_speed: 0.005,
        }
    }
}

#[derive(Resource)]
struct BasicDriver {
    up: Vec3,
    orbit_speed: f32,
    /// Orientation of the active frame, from `AlignUp`.
    frame: Quat,
}

impl Plugin for BasicDriverPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<TweenPlugin>() {
            app.add_plugins(TweenPlugin);
        }
        app.insert_resource(BasicDriver {
            up: self.up.normalize_or(Vec3::Y),
            orbit_speed: self.orbit_speed,
            frame: Quat::IDENTITY,
        })
        .add_systems(Update, handle_requests.after(ViewCubeSet));
    }
}

fn handle_requests(
    mut requests: MessageReader<CameraRequest>,
    mut driver: ResMut<BasicDriver>,
    mut commands: Commands,
    mut cams: Query<(&mut Transform, &mut Projection, &mut ViewCubeFocus)>,
) {
    for CameraRequest { camera, action } in requests.read() {
        let Ok((mut t, mut projection, mut focus)) = cams.get_mut(*camera) else {
            continue;
        };
        match action {
            CameraAction::OrbitBegin => {
                // The user takes over from any running animation.
                commands.entity(*camera).remove::<Tween>();
            }
            CameraAction::OrbitDelta(delta) => {
                commands.entity(*camera).remove::<Tween>();
                orbit(&mut t, focus.0, *delta, &driver);
            }
            CameraAction::AlignUp { frame_rotation } => driver.frame = *frame_rotation,
            CameraAction::SetProjection(to) => {
                commands.entity(*camera).remove::<Tween>();
                set_projection(&mut t, &mut projection, &mut focus, *to);
            }
            // Animated actions are the shared tween's; nothing to add.
            CameraAction::OrbitEnd | CameraAction::LookTo { .. } | CameraAction::Fit { .. } => {}
        }
    }
}

/// Orbit about the focus point: yaw about the (frame-aligned) up axis, pitch
/// about the camera's right, never past straight up / down.
fn orbit(t: &mut Transform, dist: f32, delta: Vec2, driver: &BasicDriver) {
    let up = (driver.frame * driver.up).normalize_or(Vec3::Y);
    let focus = t.translation + *t.forward() * dist;
    // Dragging right turns the scene right, i.e. the camera goes left.
    let yaw = Quat::from_axis_angle(up, -delta.x * driver.orbit_speed);
    // Rotating by +θ about the camera's right lowers the camera and reduces
    // the angle between its forward and `up` by θ; keep that angle in range.
    let angle = t.forward().angle_between(up);
    let margin = 1e-3;
    let pitch_angle = (-delta.y * driver.orbit_speed)
        .clamp(angle - (std::f32::consts::PI - margin), angle - margin);
    let pitch = Quat::from_axis_angle(*t.right(), pitch_angle);
    let rotation = yaw * pitch;
    t.translation = focus + rotation * (t.translation - focus);
    t.rotation = (rotation * t.rotation).normalize();
}

/// Switch projection, keeping what's at the focus point the same size.
fn set_projection(
    t: &mut Transform,
    projection: &mut Projection,
    focus: &mut ViewCubeFocus,
    to: ViewProjection,
) {
    match (&*projection, to) {
        (Projection::Perspective(p), ViewProjection::Orthographic) => {
            // The visible height at the focus plane becomes the ortho height.
            let height = 2.0 * focus.0 * (p.fov * 0.5).tan();
            *projection = Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: height,
                },
                ..OrthographicProjection::default_3d()
            });
        }
        (Projection::Orthographic(o), ViewProjection::Perspective) => {
            let p = PerspectiveProjection::default();
            // Back off until the perspective frustum shows the same height.
            let dist = o.area.height() / (2.0 * (p.fov * 0.5).tan());
            let focus_point = t.translation + *t.forward() * focus.0;
            t.translation = focus_point - *t.forward() * dist;
            focus.0 = dist;
            *projection = Projection::Perspective(p);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::ViewCubeTarget;

    fn app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<CameraRequest>()
            .add_message::<bevy::window::RequestRedraw>()
            .add_plugins(BasicDriverPlugin::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)));
        let cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
                ViewCubeFocus(10.0),
                ViewCubeTarget,
            ))
            .id();
        (app, cam)
    }

    fn send(app: &mut App, camera: Entity, action: CameraAction) {
        app.world_mut()
            .resource_mut::<Messages<CameraRequest>>()
            .write(CameraRequest { camera, action });
    }

    #[test]
    fn look_to_ends_facing_target_about_the_focus() {
        let (mut app, cam) = app();
        let facing = Dir3::new(Vec3::NEG_Y).unwrap();
        let up = Dir3::new(Vec3::NEG_Z).unwrap();
        send(&mut app, cam, CameraAction::LookTo { facing, up });
        for _ in 0..8 {
            app.update();
        }
        let t = app.world().get::<Transform>(cam).unwrap();
        assert!(t.forward().distance(Vec3::NEG_Y) < 1e-3, "{:?}", t.forward());
        assert!(t.up().distance(Vec3::NEG_Z) < 1e-3, "{:?}", t.up());
        // Still 10 away from the focus point at the origin.
        assert!((t.translation.length() - 10.0).abs() < 1e-2, "{:?}", t.translation);
        assert!(app.world().get::<Tween>(cam).is_none(), "tween should finish");
    }

    #[test]
    fn orbit_drag_turns_scene_with_pointer_and_clamps_pitch() {
        let (mut app, cam) = app();
        // Drag right: the camera moves left (the scene turns right).
        send(&mut app, cam, CameraAction::OrbitDelta(Vec2::new(100.0, 0.0)));
        app.update();
        let t = *app.world().get::<Transform>(cam).unwrap();
        assert!(t.translation.x < -1.0, "{:?}", t.translation);
        assert!((t.translation.length() - 10.0).abs() < 1e-3);
        // A huge drag down never flips over the top.
        send(&mut app, cam, CameraAction::OrbitDelta(Vec2::new(0.0, -1e6)));
        app.update();
        let t = *app.world().get::<Transform>(cam).unwrap();
        assert!(t.translation.is_finite());
        assert!(t.forward().dot(Vec3::Y).abs() < 1.0);
        assert!(t.up().y > 0.0, "camera rolled over: {:?}", t.up());
    }

    #[test]
    fn orbit_cancels_a_running_look_to() {
        let (mut app, cam) = app();
        let facing = Dir3::new(Vec3::NEG_X).unwrap();
        send(
            &mut app,
            cam,
            CameraAction::LookTo {
                facing,
                up: Dir3::Y,
            },
        );
        app.update();
        assert!(app.world().get::<Tween>(cam).is_some());
        send(&mut app, cam, CameraAction::OrbitBegin);
        app.update();
        assert!(app.world().get::<Tween>(cam).is_none());
    }

    #[test]
    fn projection_round_trip_keeps_framing() {
        let (mut app, cam) = app();
        send(&mut app, cam, CameraAction::SetProjection(ViewProjection::Orthographic));
        app.update();
        let height = match app.world().get::<Projection>(cam).unwrap() {
            Projection::Orthographic(o) => match o.scaling_mode {
                bevy::camera::ScalingMode::FixedVertical { viewport_height } => viewport_height,
                _ => panic!("unexpected scaling mode"),
            },
            _ => panic!("not orthographic"),
        };
        let fov = PerspectiveProjection::default().fov;
        assert!((height - 2.0 * 10.0 * (fov * 0.5).tan()).abs() < 1e-3, "{height}");
        send(&mut app, cam, CameraAction::SetProjection(ViewProjection::Perspective));
        app.update();
        assert!(matches!(
            app.world().get::<Projection>(cam).unwrap(),
            Projection::Perspective(_)
        ));
    }
}
