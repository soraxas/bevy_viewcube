//! Camera-agnostic demo: a plain `Camera3d` — no `bevy_editor_cam` — driven by
//! the dependency-free [`BasicDriverPlugin`]. Drag the cube to orbit, click a
//! face / edge / corner to look along it, click the home button for the iso
//! view (double-click: perspective ↔ orthographic), and the fit button to frame
//! the scene.
//!
//! The basic driver only acts on what the cube asks for; it adds no mouse
//! controls of its own. Use your own for the rest of the viewport.
//!
//! Run with: `cargo run --example builtin --no-default-features --features ui`
//! (or plain `cargo run --example builtin`).
//!
//! Set `VIEW_CUBE_SHOT=/tmp/cube.png` to capture a screenshot at frame 90 and
//! exit.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_viewcube::{
    driver::BasicDriverPlugin, CubeCorner, ViewCubeConfig, ViewCubePlugin, ViewCubeSettings,
    ViewCubeTarget,
};

const CUBE_LAYER: usize = 20;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        // Face clicks need mesh picking (not in DefaultPlugins).
        .add_plugins(bevy::picking::mesh_picking::MeshPickingPlugin)
        .add_plugins(BasicDriverPlugin::default())
        .add_plugins(ViewCubePlugin {
            config: ViewCubeConfig {
                corner: CubeCorner::TopRight,
                ..default()
            },
            settings: ViewCubeSettings {
                render_layer: CUBE_LAYER,
                ..default()
            },
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (drive_cube_config, maybe_screenshot))
        .run();
}

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
    // A bare camera: no controller component at all.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(6.0, 5.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        ViewCubeTarget,
    ));
}

/// The host owns `ViewCubeConfig`: keep it active over the whole window.
fn drive_cube_config(windows: Query<&Window>, mut config: ResMut<ViewCubeConfig>) {
    let Some(window) = windows.iter().next() else {
        return;
    };
    let size = window.physical_size();
    if !config.active {
        config.active = true;
    }
    if config.panel_size != size {
        config.panel_size = size;
    }
}
