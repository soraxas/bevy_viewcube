//! Minimal standalone demo of [`bevy_viewcube`].
//!
//! Orbit the scene camera with the right mouse button (pan: middle, zoom:
//! wheel) and watch the view cube in the top-right corner mirror the
//! orientation. Click a cube face to snap to that view.
//!
//! Run with: `cargo run --example basic`
//!
//! Set `VIEW_CUBE_SHOT=/tmp/cube.png` to capture a screenshot at frame 90 and
//! exit — useful for verifying the cube renders without a manual session.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_editor_cam::prelude::*;
use bevy_viewcube::{
    driver::EditorCamDriverPlugin, ViewCubeSettings, ViewCubeStyle,CubeCorner, ViewCubeConfig, ViewCubePlugin, ViewCubeTarget};

/// Render layer the cube lives on — kept off the default layer 0 (the scene).
const CUBE_LAYER: usize = 20;
const STYLE: ViewCubeStyle = ViewCubeStyle::Cad;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        // Editor camera: orbit/pan/zoom + the LookTo animation the cube drives.
        // Added before `ViewCubePlugin` so the latter's `is_plugin_added`
        // guard sees LookTo already present and skips re-adding it.
        .add_plugins(DefaultEditorCamPlugins)
        // Face clicks need mesh picking (not in DefaultPlugins).
        .add_plugins(bevy::picking::mesh_picking::MeshPickingPlugin)
        .add_plugins(EditorCamDriverPlugin::default())
        .add_plugins(ViewCubePlugin {
            config: ViewCubeConfig {
                corner: CubeCorner::TopRight,
                ..default()
            },
            settings: ViewCubeSettings {
                render_layer: CUBE_LAYER,
                style: STYLE,
                ..default()
            },
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (drive_cube_config, maybe_screenshot))
        .run();
}

/// When `VIEW_CUBE_SHOT` is set, grab a screenshot at frame 90 and exit.
fn maybe_screenshot(
    mut commands: Commands,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(path) = std::env::var("VIEW_CUBE_SHOT") else {
        return;
    };
    *frame += 1;
    if *frame == 90 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    } else if *frame == 95 {
        exit.write(AppExit::Success);
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A little scene to orbit: a ground plane and a few boxes.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(10.0, 10.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.3, 0.3, 0.35))),
    ));
    for (i, x) in [-2.0_f32, 0.0, 2.0].into_iter().enumerate() {
        let h = 1.0 + i as f32 * 0.8;
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(1.0, h, 1.0))),
            MeshMaterial3d(materials.add(Color::srgb(0.6, 0.4 + 0.15 * i as f32, 0.3))),
            Transform::from_xyz(x, h * 0.5, 0.0),
        ));
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // The scene camera: an EditorCam, tagged as the cube's target.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(6.0, 5.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        EditorCam {
            orbit_constraint: OrbitConstraint::Fixed {
                up: bevy::math::DVec3::Y,
                can_pass_tdc: false,
            },
            ..default()
        },
        ViewCubeTarget,
    ));
}

/// The host owns `ViewCubeConfig`. Here we keep it active and pin its rect to
/// the whole window (a real app would use its viewport/panel rect).
fn drive_cube_config(windows: Query<&Window>, mut config: ResMut<ViewCubeConfig>) {
    let Some(window) = windows.iter().next() else {
        return;
    };
    let size = window.physical_size();
    if !config.active {
        config.active = true;
    }
    if config.origin != UVec2::ZERO {
        config.origin = UVec2::ZERO;
    }
    if config.panel_size != size {
        config.panel_size = size;
    }
}
