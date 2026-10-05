//! Zoom-to-fit: frame the interesting objects without changing the view
//! direction. The cube itself knows nothing about the host's scene, so the host
//! says what to frame. In order of priority:
//!
//! 1. entities tagged [`FitTarget`] (typically the selection);
//! 2. the active coordinate frame's content ([`ViewCubeFrame::content`]);
//! 3. everything with a bounding box.

use bevy::prelude::*;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;

use crate::driver::{CameraAction, CameraRequest, ViewCubeFocus};
use crate::{
    CubeSettings, FitSource, FitStarted, FitView, FrameChanged, ViewCubeConfig, ViewCubeFrame, ViewCubeFrames,
    ViewCubeTarget,
};

/// Marks entities the fit button should frame (e.g. the current selection).
/// An entity with an [`Aabb`] (every mesh gets one) is framed by its box; one
/// without is treated as a point at its translation.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct FitTarget;

/// Smaller than this (world units) the targets count as a single point: center
/// it, keep the distance.
const POINT_EPSILON: f32 = 1e-4;

/// World-space points of an entity's bounds: the 8 corners of its [`Aabb`], or
/// its translation when it has none.
fn push_bounds(points: &mut Vec<Vec3>, gt: &GlobalTransform, aabb: Option<&Aabb>) {
    match aabb {
        Some(aabb) => {
            let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            for i in 0..8u32 {
                let sign = Vec3::new(
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                );
                points.push(gt.transform_point(c + h * sign));
            }
        }
        None => points.push(gt.translation()),
    }
}

/// Where the camera should go to frame `points` along its current view.
///
/// Returns `(position, ortho_scale_factor, distance)`. Points are projected
/// onto the camera's axes, so the fit is tight to the *view*, not to a
/// world-aligned box: a rotated building is framed by its actual silhouette.
fn solve_fit(
    cam: &Transform,
    projection: &Projection,
    anchor_depth: f32,
    points: &[Vec3],
    padding: f32,
) -> Option<(Vec3, f32, f32)> {
    let (r, u, f) = (*cam.right(), *cam.up(), *cam.forward());
    let coords = |p: Vec3| Vec3::new(p.dot(r), p.dot(u), p.dot(f));
    let first = coords(*points.first()?);
    let (mut lo, mut hi) = (first, first);
    for p in &points[1..] {
        let c = coords(*p);
        lo = lo.min(c);
        hi = hi.max(c);
    }
    let mid = (lo + hi) * 0.5;
    let center = r * mid.x + u * mid.y + f * mid.z;
    let half = (hi - lo) * 0.5;

    // A single point (or a degenerate pile): just put it in view.
    if half.length() < POINT_EPSILON {
        return Some((center - f * anchor_depth, 1.0, anchor_depth));
    }
    let pad = 1.0 + padding;
    match projection {
        Projection::Perspective(p) => {
            let tan_y = (p.fov * 0.5).tan();
            let tan_x = tan_y * p.aspect_ratio;
            // Each point must lie inside the frustum: with the camera `d` in
            // front of the box center, a point at relative depth `dz` sits at
            // depth `d + dz`, so |x| <= (d + dz) * tan_x, likewise for y.
            let dist = points
                .iter()
                .map(|p| {
                    let c = coords(*p) - mid;
                    ((c.x.abs() * pad / tan_x) - c.z).max((c.y.abs() * pad / tan_y) - c.z)
                })
                .fold(f32::MIN, f32::max)
                .max(half.z * 1.05 + 0.05);
            Some((center - f * dist, 1.0, dist))
        }
        Projection::Orthographic(o) => {
            let area = o.area.size();
            let factor = ((half.x * 2.0 * pad) / area.x).max((half.y * 2.0 * pad) / area.y);
            // Distance doesn't change the picture; keep it, but stay clear of
            // the box so nothing is behind the camera.
            let dist = anchor_depth.max(half.z + 1.0);
            Some((center - f * dist, factor, dist))
        }
        Projection::Custom(_) => None,
    }
}

/// Fit to the new frame's content whenever the active frame changes (opt in
/// via [`ViewCubeConfig::fit_on_frame_change`]).
pub(crate) fn fit_on_frame_change(
    mut changed: MessageReader<FrameChanged>,
    config: Res<ViewCubeConfig>,
    mut fit: MessageWriter<FitView>,
) {
    if changed.read().count() > 0 && config.fit_on_frame_change {
        fit.write(FitView);
    }
}

/// Collect world-space points for `root` and its descendants. Only entities
/// with an [`Aabb`] contribute their box; if none has one, the root's
/// translation counts as a point, so an empty group still has a place.
fn push_subtree(
    points: &mut Vec<Vec3>,
    root: Entity,
    bounds: &Query<(&GlobalTransform, Option<&Aabb>)>,
    children: &Query<&Children>,
) {
    let before = points.len();
    for entity in std::iter::once(root).chain(children.iter_descendants(root)) {
        if let Ok((gt, Some(aabb))) = bounds.get(entity) {
            push_bounds(points, gt, Some(aabb));
        }
    }
    if points.len() == before {
        if let Ok((gt, _)) = bounds.get(root) {
            push_bounds(points, gt, None);
        }
    }
}

/// Handle [`FitView`]: compute where the camera must go to frame the targets
/// and ask the driver to move it there ([`CameraAction::Fit`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn fit_view(
    mut requests: MessageReader<FitView>,
    config: Res<ViewCubeConfig>,
    settings: Res<CubeSettings>,
    frames: Res<ViewCubeFrames>,
    mut out: MessageWriter<CameraRequest>,
    mut started: MessageWriter<FitStarted>,
    cam: Query<(Entity, &Transform, &Projection, &ViewCubeFocus), With<ViewCubeTarget>>,
    tagged: Query<Entity, With<FitTarget>>,
    bounds: Query<(&GlobalTransform, Option<&Aabb>)>,
    children: Query<&Children>,
    scene: Query<(&GlobalTransform, &Aabb, Option<&RenderLayers>)>,
) {
    if requests.read().count() == 0 {
        return;
    }
    let Ok((camera, transform, projection, focus)) = cam.single() else {
        return;
    };
    let mut points = Vec::new();
    let mut source = FitSource::Selection;
    // 1. The selection.
    for entity in &tagged {
        if let Ok((gt, aabb)) = bounds.get(entity) {
            push_bounds(&mut points, gt, aabb);
        }
    }
    // 2. The active frame's own content.
    if points.is_empty() {
        let content = frames.frames.get(frames.active).and_then(|f: &ViewCubeFrame| f.content);
        if let Some(root) = content {
            push_subtree(&mut points, root, &bounds, &children);
            source = FitSource::Frame(frames.active);
        }
    }
    // 3. Everything, minus the cube's own meshes.
    if points.is_empty() {
        source = FitSource::Scene;
        let cube_layer = RenderLayers::layer(settings.0.render_layer);
        for (gt, aabb, layers) in &scene {
            if layers.is_some_and(|l| l.intersects(&cube_layer)) {
                continue;
            }
            push_bounds(&mut points, gt, Some(aabb));
        }
    }
    let Some((position, factor, focus_distance)) =
        solve_fit(transform, projection, focus.0, &points, config.fit_padding)
    else {
        return;
    };
    started.write(FitStarted { camera, source });
    out.write(CameraRequest {
        camera,
        action: CameraAction::Fit {
            position,
            ortho_scale: match projection {
                Projection::Orthographic(o) => Some(o.scale * factor),
                _ => None,
            },
            focus_distance,
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam_at(pos: Vec3) -> Transform {
        Transform::from_translation(pos).looking_at(Vec3::ZERO, Vec3::Y)
    }

    #[test]
    fn single_point_keeps_distance_and_centers() {
        let cam = cam_at(Vec3::new(0.0, 0.0, 10.0));
        let proj = Projection::Perspective(PerspectiveProjection::default());
        let (pos, factor, dist) =
            solve_fit(&cam, &proj, 10.0, &[Vec3::new(3.0, 2.0, 0.0)], 0.1).unwrap();
        assert_eq!(factor, 1.0);
        assert!((dist - 10.0).abs() < 1e-4);
        // Camera slid over to look straight at the point, same distance.
        assert!(pos.distance(Vec3::new(3.0, 2.0, 10.0)) < 1e-4, "{pos:?}");
    }

    #[test]
    fn perspective_fit_contains_all_corners() {
        let cam = cam_at(Vec3::new(0.0, 0.0, 50.0));
        let p = PerspectiveProjection::default();
        let proj = Projection::Perspective(p.clone());
        let mut pts = Vec::new();
        push_bounds(
            &mut pts,
            &GlobalTransform::from(Transform::from_xyz(5.0, 1.0, 0.0)),
            Some(&Aabb::from_min_max(Vec3::splat(-1.0), Vec3::new(3.0, 1.0, 2.0))),
        );
        let (pos, _, _) = solve_fit(&cam, &proj, 50.0, &pts, 0.0).unwrap();
        let fitted = Transform::from_translation(pos).with_rotation(cam.rotation);
        let tan_y = (p.fov * 0.5).tan();
        let tan_x = tan_y * p.aspect_ratio;
        let mut max_ratio: f32 = 0.0;
        for q in &pts {
            let local = fitted.compute_affine().inverse().transform_point3(*q);
            let depth = -local.z;
            assert!(depth > 0.0);
            max_ratio = max_ratio.max((local.x.abs() / (depth * tan_x)).max(local.y.abs() / (depth * tan_y)));
        }
        // Tight: the limiting corner just touches the frustum edge.
        assert!((max_ratio - 1.0).abs() < 1e-3, "{max_ratio}");
    }

    #[test]
    fn orthographic_fit_scales_to_extent() {
        let cam = cam_at(Vec3::new(0.0, 0.0, 20.0));
        let mut o = OrthographicProjection::default_3d();
        o.area = Rect::from_center_size(Vec2::ZERO, Vec2::new(8.0, 4.0));
        let proj = Projection::Orthographic(o);
        let pts = [Vec3::new(-4.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        let (_, factor, _) = solve_fit(&cam, &proj, 20.0, &pts, 0.5).unwrap();
        // 8 wide * 1.5 padding = 12 needed vs 8 visible.
        assert!((factor - 1.5).abs() < 1e-4, "{factor}");
    }

    // --- fit_view source selection (selection > frame content > scene) ---

    use crate::driver::CameraRequest as Req;
    use crate::{ViewCubeFrame as Frame, ViewCubeSettings};

    fn fit_app(frames: ViewCubeFrames) -> (App, Entity) {
        let mut app = App::new();
        app.add_message::<Req>()
            .add_message::<FitStarted>()
            .add_message::<FitView>()
            .insert_resource(ViewCubeConfig::default())
            .insert_resource(CubeSettings(ViewCubeSettings::default()))
            .insert_resource(frames)
            .add_systems(Update, fit_view);
        let cam = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 100.0).looking_at(Vec3::ZERO, Vec3::Y),
                Projection::Perspective(PerspectiveProjection::default()),
                ViewCubeFocus(100.0),
                ViewCubeTarget,
            ))
            .id();
        (app, cam)
    }

    fn boxed(app: &mut App, x: f32) -> Entity {
        app.world_mut()
            .spawn((
                GlobalTransform::from(Transform::from_xyz(x, 0.0, 0.0)),
                Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
            ))
            .id()
    }

    /// Camera x the fit asked for.
    fn fitted_x(app: &mut App) -> f32 {
        app.world_mut().write_message(FitView);
        app.update();
        let msgs = app.world().resource::<Messages<Req>>();
        let mut cursor = msgs.get_cursor();
        match cursor.read(msgs).last().expect("a fit request").action {
            CameraAction::Fit { position, .. } => position.x,
            _ => panic!("not a fit"),
        }
    }

    /// What the most recent fit said it framed.
    fn last_source(app: &App) -> FitSource {
        let msgs = app.world().resource::<Messages<FitStarted>>();
        let mut cursor = msgs.get_cursor();
        cursor.read(msgs).last().expect("a FitStarted").source
    }

    #[test]
    fn fits_whole_scene_without_frame_content() {
        let (mut app, _) = fit_app(ViewCubeFrames::default());
        boxed(&mut app, -50.0);
        boxed(&mut app, 50.0);
        assert!(fitted_x(&mut app).abs() < 1e-2, "centered on both");
    }

    #[test]
    fn active_frame_content_limits_the_fit() {
        let mut frames = ViewCubeFrames::default();
        frames.frames = vec![Frame::new("World", Quat::IDENTITY), Frame::new("A", Quat::IDENTITY)];
        let (mut app, _) = fit_app(frames);
        let a = boxed(&mut app, -50.0);
        boxed(&mut app, 50.0);
        app.world_mut().resource_mut::<ViewCubeFrames>().frames[1].content = Some(a);
        // World active: everything.
        assert!(fitted_x(&mut app).abs() < 1e-2);
        assert_eq!(last_source(&app), FitSource::Scene);
        // Frame A active: just A.
        app.world_mut().resource_mut::<ViewCubeFrames>().active = 1;
        assert!((fitted_x(&mut app) + 50.0).abs() < 1e-2);
        assert_eq!(last_source(&app), FitSource::Frame(1));
    }

    #[test]
    fn selection_beats_frame_content_and_children_count() {
        let mut frames = ViewCubeFrames::default();
        frames.frames = vec![Frame::new("A", Quat::IDENTITY)];
        let (mut app, _) = fit_app(frames);
        // Content root with a child box; the root itself has no Aabb.
        let root = app
            .world_mut()
            .spawn(GlobalTransform::from(Transform::from_xyz(-50.0, 0.0, 0.0)))
            .id();
        let child = boxed(&mut app, -50.0);
        app.world_mut().entity_mut(child).insert(ChildOf(root));
        app.world_mut().resource_mut::<ViewCubeFrames>().frames[0].content = Some(root);
        assert!((fitted_x(&mut app) + 50.0).abs() < 1e-2, "frame content via child");
        // A tagged selection elsewhere wins.
        let selected = boxed(&mut app, 30.0);
        app.world_mut().entity_mut(selected).insert(FitTarget);
        assert!((fitted_x(&mut app) - 30.0).abs() < 1e-2, "selection wins");
        assert_eq!(last_source(&app), FitSource::Selection);
    }
}
