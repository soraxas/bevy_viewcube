//! Gizmo geometry: the overlay camera, the mirror pivot, and the style-specific
//! body — either the AutoCAD-style solid cube (see `cad`) or a Blender-style
//! open axis gizmo of three colored arrows (shaft + pointy conical head +
//! letter), which needs no solid body since the depth buffer sorts the arrows
//! naturally from any orientation.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::NoIndirectDrawing;
use bevy::camera::visibility::RenderLayers;
use crate::text3d::Caption;

use crate::{CubeSettings, ViewCubeStyle};

/// Marker on the dedicated camera that renders the view cube.
#[derive(Component, Debug)]
pub struct CubeCamera;

/// Marker on the cube pivot whose rotation mirrors the target camera.
#[derive(Component, Debug)]
pub struct CubePivot;

/// Marker + snap preset on each clickable region: an axis arrowhead, or a
/// face / edge / corner of the CAD cube.
#[derive(Component, Debug, Clone, Copy)]
pub struct CubeFace(pub ViewPreset);

/// Resting and hover materials of a CAD cube region; swapped on pointer
/// over / out.
#[derive(Component, Debug, Clone)]
pub struct CubeHover {
    pub base: Handle<StandardMaterial>,
    pub hover: Handle<StandardMaterial>,
}

/// Root of the frame's local-axes triad.
#[derive(Component, Debug)]
pub struct CubeLocalAxes;

/// Marks a [`CubeLocalAxes`] triad pinned to a corner of the cube viewport
/// (a child of the cube camera); `sync_viewport_axes` places and turns it.
#[derive(Component, Debug)]
pub struct CubeViewportAxes(pub crate::CubeCorner);

/// An axis letter mesh, billboarded toward the cube camera each frame. The
/// pivot rotates the gizmo, which would carry the letter edge-on; the
/// billboard system cancels that and nudges the letter toward the camera. The
/// stored `base` is the letter's anchor position in pivot space (out near the
/// arrow tip); the camera-facing offset is added on top each frame.
#[derive(Component, Debug)]
pub struct CubeAxisLabel {
    pub base: Vec3,
}

/// Canonical camera orientations. The facing direction is the way the camera
/// *looks* (toward the scene); the snap helper resolves the up vector against
/// the target's orbit constraint. Clicking the +X ball looks *from* +X (i.e.
/// faces -X), and so on for each of the six axis directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    /// Look from +Y, straight down (-Y) onto the ground plane.
    Top,
    /// Look from -Y, straight up (+Y).
    Bottom,
    /// Look from +Z toward -Z (the default editing front).
    Front,
    /// Look from -Z toward +Z.
    Back,
    /// Look from +X toward -X.
    Right,
    /// Look from -X toward +X.
    Left,
    /// 3/4 isometric — the resting editor pose.
    Iso,
    /// Look from the cube edge or corner at this offset (each component in
    /// `-1..=1`, not all zero) toward the origin, e.g. `(1, 1, 1)` is the
    /// +X/+Y/+Z corner. Face offsets are better built via
    /// [`ViewPreset::from_offset`], which yields the named face presets.
    Cube(IVec3),
}

impl ViewPreset {
    /// Preset for the cube region at `offset` (components in `-1..=1`): a face
    /// maps to its named preset, an edge or corner to [`ViewPreset::Cube`].
    pub fn from_offset(offset: IVec3) -> Self {
        match offset.to_array() {
            [1, 0, 0] => ViewPreset::Right,
            [-1, 0, 0] => ViewPreset::Left,
            [0, 1, 0] => ViewPreset::Top,
            [0, -1, 0] => ViewPreset::Bottom,
            [0, 0, 1] => ViewPreset::Front,
            [0, 0, -1] => ViewPreset::Back,
            _ => ViewPreset::Cube(offset),
        }
    }

    /// Direction the camera faces (normalized) for this preset.
    pub fn facing(self) -> Vec3 {
        match self {
            ViewPreset::Top => Vec3::NEG_Y,
            ViewPreset::Bottom => Vec3::Y,
            ViewPreset::Front => Vec3::NEG_Z,
            ViewPreset::Back => Vec3::Z,
            ViewPreset::Right => Vec3::NEG_X,
            ViewPreset::Left => Vec3::X,
            ViewPreset::Iso => Vec3::new(-0.6, -0.55, -1.0).normalize(),
            ViewPreset::Cube(offset) => -offset.as_vec3().normalize_or(Vec3::Z),
        }
    }
}

/// Uniform scale applied to an em-normalized axis letter so a cap reads as
/// roughly the ball's diameter.
const LABEL_SCALE: f32 = 0.26;

/// Distance (cube-space units) the letter sits in front of its ball, toward
/// the cube camera. Applied in world space by `billboard_axis_labels`.
pub(crate) const LABEL_CAM_OFFSET: f32 = 0.22;

/// One axis arrow: direction from the center, snap preset, and color. The
/// label text is resolved from [`ViewCubeLabels`] at spawn (host-configurable),
/// not baked in here. Only the three positive axes are drawn.
struct AxisArrow {
    dir: Vec3,
    preset: ViewPreset,
    color: Color,
}

pub(crate) const RED: Color = Color::srgb(0.90, 0.30, 0.36);
pub(crate) const GREEN: Color = Color::srgb(0.55, 0.80, 0.30);
pub(crate) const BLUE: Color = Color::srgb(0.30, 0.60, 0.90);

/// The three axis arrows: one pointy head per axis line (X / Y / Z).
fn axis_arrows() -> [AxisArrow; 3] {
    [
        AxisArrow {
            dir: Vec3::X,
            preset: ViewPreset::Right,
            color: RED,
        },
        AxisArrow {
            dir: Vec3::Y,
            preset: ViewPreset::Top,
            color: GREEN,
        },
        AxisArrow {
            dir: Vec3::Z,
            preset: ViewPreset::Front,
            color: BLUE,
        },
    ]
}

/// Distance (cube-space units) the arrow *tip* reaches from the gizmo center.
const ARROW_TIP: f32 = 1.0;
/// Length of the conical arrowhead along its axis.
const HEAD_LEN: f32 = 0.34;
/// Base radius of the conical arrowhead.
const HEAD_RADIUS: f32 = 0.16;
/// Half-thickness (radius) of an axis line cylinder.
const STUB_RADIUS: f32 = 0.035;

/// Spawn the gizmo camera, pivot, three axis lines, and three pointy pickable
/// arrowheads (with letters) once at startup. Inactive until
/// [`ViewCubeConfig::active`](crate::ViewCubeConfig::active) is set (handled by `sync_cube_camera_active`).
pub(crate) fn setup_view_cube(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    settings: Res<CubeSettings>,
) {
    let settings = &settings.0;
    let labels = &settings.axis_labels;
    let layer = RenderLayers::layer(settings.render_layer);
    let font = match &settings.font {
        Some(path) => asset_server.load(path.clone()),
        None => Handle::default(),
    };

    // Dedicated camera: looks at the cube from a fixed iso angle. The cube
    // pivot rotates to mirror the target camera, so this camera stays put.
    let cad = settings.style == ViewCubeStyle::Cad;
    let cam = commands.spawn((
        Camera3d::default(),
        // The default `TonyMcMapFace` tonemap needs a LUT feature that may not
        // be enabled and renders broken (magenta) without it; the cube is
        // already authored in display colors, so skip tonemapping.
        Tonemapping::None,
        Camera {
            // High order so it overlays the host scene cameras.
            order: 10,
            clear_color: ClearColorConfig::None,
            is_active: false,
            ..default()
        },
        // Fixed viewpoint; the cube's *rotation* conveys orientation. The CAD
        // cube is viewed dead-on so a standard view shows exactly one face;
        // the axis gizmo gets a slight elevation so arrows never line up
        // edge-on.
        Transform::from_xyz(0.0, if cad { 0.0 } else { 0.6 }, 4.0)
            .looking_at(Vec3::ZERO, Vec3::Y),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                // The CAD cube's corners reach further than the axis arrows.
                viewport_height: if cad { crate::cad::CUBE_HEIGHT } else { 2.9 },
            },
            ..OrthographicProjection::default_3d()
        }),
        layer.clone(),
        NoIndirectDrawing,
        CubeCamera,
        Name::new("ViewCubeCamera"),
    )).id();

    // Pivot mirrors the target camera orientation. Children: the six axis
    // balls and their stubs. No solid cube body — a Blender-style open axis
    // gizmo, so the depth buffer sorts the balls naturally (the ones facing
    // the camera draw in front, the ones behind draw behind) with nothing to
    // overlay.
    let pivot = commands
        .spawn((
            CubePivot,
            Transform::default(),
            Visibility::Hidden,
            layer.clone(),
            Name::new("ViewCubePivot"),
        ))
        .id();

    if cad {
        crate::cad::spawn_cad_cube(
            &mut commands,
            &mut meshes,
            &mut materials,
            &font,
            &settings.face_labels,
            labels,
            settings.axes,
            &layer,
            pivot,
            cam,
        );
        return;
    }

    // Unit-height cylinder + cone (both +Y-aligned), reused for every axis and
    // oriented along `dir`. The line runs origin → arrowhead base; the cone is
    // the pointy head reaching out to the tip.
    let line_mesh = meshes.add(Cylinder::new(STUB_RADIUS, 1.0));
    let head_mesh = meshes.add(Cone {
        radius: HEAD_RADIUS,
        height: HEAD_LEN,
    });

    // Length of the shaft: from the origin to the base of the cone.
    let line_len = ARROW_TIP - HEAD_LEN;

    for arrow in axis_arrows() {
        let axis = arrow.dir;
        // Host-configurable label for this axis (defaults X/Y/Z).
        let letter: &str = if axis == Vec3::X {
            &labels.x
        } else if axis == Vec3::Y {
            &labels.y
        } else {
            &labels.z
        };
        let mat = materials.add(StandardMaterial {
            base_color: arrow.color,
            unlit: true,
            ..default()
        });
        // Orient +Y along the axis.
        let rot = Quat::from_rotation_arc(Vec3::Y, axis);

        // Shaft: cylinder from origin out to the arrowhead base.
        commands.spawn((
            Mesh3d(line_mesh.clone()),
            MeshMaterial3d(mat.clone()),
            Transform {
                translation: axis * (line_len * 0.5),
                rotation: rot,
                scale: Vec3::new(1.0, line_len, 1.0),
            },
            layer.clone(),
            ChildOf(pivot),
            Name::new("ViewCubeAxisLine"),
        ));

        // Pointy head: a cone whose apex points outward along the axis, sitting
        // on the end of the shaft. This is the pickable snap target.
        let head_center = axis * (line_len + HEAD_LEN * 0.5);
        commands
            .spawn((
                Mesh3d(head_mesh.clone()),
                MeshMaterial3d(mat),
                Transform {
                    translation: head_center,
                    rotation: rot,
                    scale: Vec3::ONE,
                },
                layer.clone(),
                bevy::picking::Pickable::default(),
                CubeFace(arrow.preset),
                ChildOf(pivot),
                Name::new(format!("ViewCubeArrow:{letter}")),
            ))
            .observe(crate::systems::on_cube_face_click);

        // Letter floats just past the tip. It's a child of the *pivot* (not the
        // head) so it carries no axis rotation — the billboard system only has
        // to cancel the pivot's rotation to keep it facing the camera, and it
        // adds the camera-facing offset to `base` each frame.
        let label_base = axis * (ARROW_TIP + 0.22);
        commands.spawn((
            Caption::new(
                letter.to_string(),
                font.clone(),
                Color::srgb(0.97, 0.97, 0.98),
                32.0,
                0.045,
            ),
            // Caption quads are em-normalized (1 em = 1.0 unit, a cap ≈ 0.7);
            // ~0.26 reads as a small tag near the arrow tip.
            Transform::from_translation(label_base).with_scale(Vec3::splat(LABEL_SCALE)),
            layer.clone(),
            CubeAxisLabel { base: label_base },
            ChildOf(pivot),
            Name::new(format!("CubeAxisLabel:{letter}")),
        ));
    }
}
