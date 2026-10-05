//! AutoCAD-style view cube: a solid cube split into 26 clickable regions
//! (6 faces, 12 edges, 8 corners) with captions printed on the faces, plus the
//! frame's local-axes triad. The home button and frame dropdown are Bevy UI
//! (see `ui`).

use bevy::prelude::*;
use bevy::camera::visibility::RenderLayers;
use crate::text3d::Caption;

use crate::cube::{CubeAxisLabel, CubeFace, CubeHover, CubeLocalAxes, CubeViewportAxes};
use crate::systems::{
    on_cube_drag, on_cube_drag_end, on_cube_drag_start, on_cube_face_click, on_cube_hover_end,
    on_cube_hover_start, on_cube_press,
};
use crate::{AxesPlacement, ViewCubeFaceLabels, ViewCubeLabels, ViewPreset};

/// Width of a face's center region (cube-space units).
const CORE: f32 = 1.4;
/// Width of an edge / corner strip. `CORE + 2 * EDGE` is the full cube side.
const EDGE: f32 = 0.3;
/// Gap left between regions so their outlines read as seams.
const GAP: f32 = 0.04;
/// Viewport height (cube-space units) of the cube camera, without / with room
/// at the top for the frame-selector button (a UI node over the viewport).
pub(crate) const CUBE_HEIGHT: f32 = 3.4;
pub(crate) const SELECTOR_HEIGHT: f32 = 4.4;

/// Whole-gizmo scale, leaving room in the viewport for the corners at any
/// orientation.
const CUBE_SCALE: f32 = 0.85;

const FACE_COLOR: Color = Color::srgb(0.80, 0.82, 0.86);
const EDGE_COLOR: Color = Color::srgb(0.62, 0.65, 0.71);
const HOVER_COLOR: Color = Color::srgb(0.30, 0.58, 0.95);
const CAPTION_COLOR: Color = Color::srgb(0.22, 0.25, 0.30);
/// Axis letters.
const LABEL_COLOR: Color = Color::srgb(0.97, 0.97, 0.98);
/// Rasterisation sizes (px per em) of face captions and axis letters, a little
/// over their on-screen size.
const CAPTION_PX: f32 = 48.0;
const LABEL_PX: f32 = 32.0;
/// Extra stroke weight (ems) so the text holds up when shrunk to a few pixels.
const CAPTION_BOLD: f32 = 0.03;
const LABEL_BOLD: f32 = 0.045;

/// Spawn the cube regions, captions, lighting and axes triad under `pivot`
/// (regions, captions) and `camera` (a viewport-pinned triad, which doesn't
/// rotate with the cube itself).
pub(crate) fn spawn_cad_cube(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    font: &Handle<Font>,
    captions: &ViewCubeFaceLabels,
    axis_labels: &ViewCubeLabels,
    axes: AxesPlacement,
    layer: &RenderLayers,
    pivot: Entity,
    camera: Entity,
) {
    commands
        .entity(pivot)
        .insert(Transform::from_scale(Vec3::splat(CUBE_SCALE)));

    // Cube-local lighting: a fixed key light plus a soft per-camera ambient,
    // both confined to the cube's layer so the host scene is untouched. The
    // pivot rotates under the light, so each face shades by its orientation.
    commands.entity(camera).insert(AmbientLight {
        brightness: 650.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 3500.0,
            ..default()
        },
        Transform::from_xyz(-1.5, 2.5, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
        layer.clone(),
        Name::new("ViewCubeLight"),
    ));

    let paint = |materials: &mut Assets<StandardMaterial>, color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.7,
            reflectance: 0.1,
            ..default()
        })
    };
    let face_mat = paint(materials, FACE_COLOR);
    let edge_mat = paint(materials, EDGE_COLOR);
    let hover_mat = paint(materials, HOVER_COLOR);

    const STRIDE: f32 = CORE * 0.5 + EDGE * 0.5;
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let offset = IVec3::new(x, y, z);
                if offset == IVec3::ZERO {
                    continue;
                }
                let is_face = offset.abs().element_sum() == 1;
                let dims = offset.abs().as_vec3().map(|c| if c == 0.0 { CORE } else { EDGE }) - GAP;
                let base = if is_face { &face_mat } else { &edge_mat };
                commands
                    .spawn((
                        Mesh3d(meshes.add(Cuboid::new(dims.x, dims.y, dims.z))),
                        MeshMaterial3d(base.clone()),
                        Transform::from_translation(offset.as_vec3() * STRIDE),
                        layer.clone(),
                        bevy::picking::Pickable::default(),
                        CubeFace(ViewPreset::from_offset(offset)),
                        CubeHover {
                            base: base.clone(),
                            hover: hover_mat.clone(),
                        },
                        ChildOf(pivot),
                        Name::new(format!("ViewCubeRegion:{x},{y},{z}")),
                    ))
                    .observe(on_cube_face_click)
            .observe(on_cube_press)
            .observe(on_cube_drag_start)
            .observe(on_cube_drag)
            .observe(on_cube_drag_end)
                    .observe(on_cube_press)
                    .observe(on_cube_drag_start)
                    .observe(on_cube_drag)
                    .observe(on_cube_drag_end)
                    .observe(on_cube_hover_start)
                    .observe(on_cube_hover_end);
            }
        }
    }

    // Captions: (outward normal, text right, text up, text). Right/up are
    // chosen so the text reads upright with the cube in its default pose
    // (looking at FRONT, top face above), and `right × up == normal` keeps
    // each glyph quad facing outward.
    let faces = [
        (Vec3::Z, Vec3::X, Vec3::Y, &captions.front),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y, &captions.back),
        (Vec3::X, Vec3::NEG_Z, Vec3::Y, &captions.right),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y, &captions.left),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z, &captions.top),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z, &captions.bottom),
    ];
    for (normal, right, up, text) in faces {
        // Bold caps run ~0.72 em wide; fit long captions (custom ones too)
        // inside the face's center region with a little breathing room.
        let scale = (1.2 / (0.72 * text.chars().count().max(1) as f32)).min(0.38);
        commands.spawn((
            Caption::new(text.clone(), font.clone(), CAPTION_COLOR, CAPTION_PX, CAPTION_BOLD),
            Transform {
                // Just proud of the (gap-shrunk) face surface.
                translation: normal * (1.0 - GAP * 0.5 + 0.005),
                rotation: Quat::from_mat3(&Mat3::from_cols(right, up, normal)),
                scale: Vec3::splat(scale),
            },
            layer.clone(),
            // Captions sit just proud of their face; without this they win
            // the hit test and swallow clicks/drags meant for the face.
            bevy::picking::Pickable::IGNORE,
            ChildOf(pivot),
            Name::new(format!("ViewCubeCaption:{text}")),
        ));
    }

    spawn_local_axes(
        commands, meshes, materials, font, axis_labels, layer, pivot, camera, axes,
    );
}

/// Geometry of a local-axes triad, in the units of the space it's spawned in.
struct AxesLook {
    /// Where the three axes start.
    origin: Vec3,
    len: f32,
    head_len: f32,
    line_radius: f32,
    head_radius: f32,
    /// Gap from the arrow tip to its letter.
    label_gap: f32,
    label_scale: f32,
}

impl AxesLook {
    /// Attached to the cube: starts just outside its -X / -Y / +Z
    /// (front-left-bottom) corner, so in the resting pose X and Y run along
    /// visible edges and Z points at the viewer. Pivot space (scaled).
    const CUBE_CORNER: Self = Self {
        origin: Vec3::new(-1.12, -1.12, 1.12),
        len: 1.0,
        head_len: 0.16,
        line_radius: 0.028,
        head_radius: 0.07,
        label_gap: 0.16,
        label_scale: 0.2,
    };
    /// Pinned in the viewport corner: compact, centered on its own origin so
    /// it fits any orientation. Camera space.
    const VIEWPORT: Self = Self {
        origin: Vec3::ZERO,
        len: 0.42,
        head_len: 0.1,
        line_radius: 0.02,
        head_radius: 0.05,
        label_gap: 0.12,
        label_scale: 0.17,
    };
}

/// Spawn the frame's local X / Y / Z axes: three colored lines with small
/// arrowheads and letters. Attached to the pivot ([`AxesPlacement::CubeCorner`])
/// they turn with the cube for free; pinned to the viewport they're children
/// of the fixed camera and `sync_viewport_axes` turns them. Either way the
/// cube is aligned to the active frame, so these are the frame's axes. Letters
/// reuse `billboard_axis_labels` via [`CubeAxisLabel`] (valid because the cube
/// camera has identity rotation, so the triad root's world rotation equals the
/// pivot's).
fn spawn_local_axes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    font: &Handle<Font>,
    labels: &ViewCubeLabels,
    layer: &RenderLayers,
    pivot: Entity,
    camera: Entity,
    placement: AxesPlacement,
) {
    let (look, parent, viewport_corner) = match placement {
        AxesPlacement::Hidden => return,
        AxesPlacement::CubeCorner => (AxesLook::CUBE_CORNER, pivot, None),
        AxesPlacement::ViewportCorner(corner) => (AxesLook::VIEWPORT, camera, Some(corner)),
    };
    let mut root = commands.spawn((
        CubeLocalAxes,
        Transform::default(),
        Visibility::default(),
        layer.clone(),
        ChildOf(parent),
        Name::new("ViewCubeLocalAxes"),
    ));
    if let Some(corner) = viewport_corner {
        root.insert(CubeViewportAxes(corner));
    }
    let root = root.id();

    let line = meshes.add(Cylinder::new(look.line_radius, 1.0));
    let head = meshes.add(Cone::new(look.head_radius, look.head_len));
    let shaft_len = look.len - look.head_len;

    for (axis, color, text) in [
        (Vec3::X, crate::cube::RED, &labels.x),
        (Vec3::Y, crate::cube::GREEN, &labels.y),
        (Vec3::Z, crate::cube::BLUE, &labels.z),
    ] {
        let mat = materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            ..default()
        });
        let rot = Quat::from_rotation_arc(Vec3::Y, axis);
        commands.spawn((
            Mesh3d(line.clone()),
            MeshMaterial3d(mat.clone()),
            Transform {
                translation: look.origin + axis * (shaft_len * 0.5),
                rotation: rot,
                scale: Vec3::new(1.0, shaft_len, 1.0),
            },
            layer.clone(),
            bevy::picking::Pickable::IGNORE,
            ChildOf(root),
            Name::new("ViewCubeLocalAxisLine"),
        ));
        commands.spawn((
            Mesh3d(head.clone()),
            MeshMaterial3d(mat),
            Transform {
                translation: look.origin + axis * (shaft_len + look.head_len * 0.5),
                rotation: rot,
                scale: Vec3::ONE,
            },
            layer.clone(),
            bevy::picking::Pickable::IGNORE,
            ChildOf(root),
            Name::new("ViewCubeLocalAxisHead"),
        ));
        let base = look.origin + axis * (look.len + look.label_gap);
        commands.spawn((
            Caption::new(text.clone(), font.clone(), LABEL_COLOR, LABEL_PX, LABEL_BOLD),
            Transform::from_translation(base).with_scale(Vec3::splat(look.label_scale)),
            layer.clone(),
            bevy::picking::Pickable::IGNORE,
            CubeAxisLabel { base },
            ChildOf(root),
            Name::new(format!("ViewCubeLocalAxisLabel:{text}")),
        ));
    }
}
