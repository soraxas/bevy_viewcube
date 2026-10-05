//! Coordinate-frame demo: the optional dropdown above the cube
//! ([`ViewCubeFrames`]). Three "buildings" sit at random orientations; click
//! the dropdown to pick World / Building A / B / C and the cube — and the views
//! its faces snap to — align to that frame. Its X / Y / Z axes (beside the
//! cube) are the frame's local axes. Click FRONT under "Building B" to look
//! straight at that building's front.
//!
//! The fit button frames the active frame's building (everything under
//! World), and picking a frame fits it too.
//!
//! Fits and projection switches flash a short message (the cube emits
//! `FitStarted` / `ProjectionChanged`; this example just listens).
//!
//! Orbit with the right mouse button (pan: middle, zoom: wheel).
//!
//! Run with: `cargo run --example frames`. Set `VIEW_CUBE_SEED=<n>` for a
//! repeatable layout, and `VIEW_CUBE_FRAME=<i>` to start on another frame.
//!
//! Set `VIEW_CUBE_SHOT=/tmp/cube.png` to capture a screenshot at frame 90 and
//! exit.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_editor_cam::prelude::*;
use bevy_viewcube::{
    driver::EditorCamDriverPlugin, ViewCubeSettings, ViewCubeStyle,
    CubeCorner, FitSource, FitStarted, FitTarget, FitView, FrameChanged, ProjectionChanged,
    ToggleProjection, ViewCubeConfig, ViewCubeFrame, ViewCubeFrames, ViewCubePlugin,
    ViewCubeTarget, ViewProjection,
};

/// Render layer the cube lives on — kept off the default layer 0 (the scene).
const CUBE_LAYER: usize = 20;
const STYLE: ViewCubeStyle = ViewCubeStyle::Cad;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                // Browser build (`examples/wasm/serve.sh`): draw into the page's
                // canvas, fill it, and keep touches from scrolling / zooming.
                // No effect on desktop.
                canvas: Some("#bevy".into()),
                fit_canvas_to_parent: true,
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
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
                // Picking a frame in the dropdown also fits that building.
                fit_on_frame_change: true,
                ..default()
            },
            settings: ViewCubeSettings {
                render_layer: CUBE_LAYER,
                style: STYLE,
                ..default()
            },
            frames: ViewCubeFrames {
                frames: std::iter::once(ViewCubeFrame::new("World", Quat::IDENTITY))
                    .chain(buildings().into_iter().map(|b| ViewCubeFrame::new(b.name, b.rotation)))
                    .collect(),
                active: std::env::var("VIEW_CUBE_FRAME")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                ..default()
            },
            ..default()
        })
        .add_systems(Startup, (setup, spawn_toast))
        .add_systems(
            Update,
            (
                drive_cube_config,
                maybe_screenshot,
                log_frame_changes,
                auto_actions,
                show_toasts,
                fade_toast,
            ),
        )
        .run();
}

/// One building: where it stands, how it's oriented, and its footprint/height.
struct Building {
    name: &'static str,
    position: Vec3,
    rotation: Quat,
    size: Vec3,
}

/// Three buildings with pseudo-random orientations: a free yaw plus a few
/// degrees of pitch/roll, so each frame differs visibly from the world axes.
/// Deterministic per run (seeded from `VIEW_CUBE_SEED`, else the clock), and
/// the same across calls so the scene and the frame list agree.
fn buildings() -> [Building; 3] {
    static SEED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let seed = *SEED.get_or_init(|| {
        std::env::var("VIEW_CUBE_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(1, |d| d.as_nanos() as u64)
            })
    });
    // xorshift64*: plenty for picking angles, no extra dependency.
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut building = |name, position, size| {
        let yaw = next() * std::f32::consts::TAU;
        let pitch = (next() - 0.5) * 30f32.to_radians();
        let roll = (next() - 0.5) * 30f32.to_radians();
        Building {
            name,
            position,
            rotation: Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll),
            size,
        }
    };
    [
        building("Building A", Vec3::new(-3.2, 0.9, 0.0), Vec3::new(1.4, 1.8, 2.2)),
        building("Building B", Vec3::new(0.0, 1.2, -0.5), Vec3::new(1.2, 2.4, 1.8)),
        building("Building C", Vec3::new(3.2, 0.7, 0.3), Vec3::new(2.0, 1.4, 1.2)),
    ]
}

/// For screenshots, do what the buttons would: `VIEW_CUBE_FIT` sends `FitView`
/// (frames the active frame's building, or everything under World; with
/// `=sel`, Building B is tagged as a [`FitTarget`] selection, which wins over
/// the frame) and `VIEW_CUBE_TOGGLE` flips the projection.
fn auto_actions(
    mut frame: Local<u32>,
    mut fit: MessageWriter<FitView>,
    mut toggle: MessageWriter<ToggleProjection>,
) {
    *frame += 1;
    if *frame == 30 && std::env::var("VIEW_CUBE_FIT").is_ok() {
        fit.write(FitView);
    }
    if *frame == 30 && std::env::var("VIEW_CUBE_TOGGLE").is_ok() {
        toggle.write(ToggleProjection);
    }
}

// --- A little OSD: the cube emits events, the host just listens. ---

/// Seconds a toast stays fully visible, then takes to fade out.
const TOAST_HOLD: f32 = 1.0;
const TOAST_FADE: f32 = 0.5;

/// The pill showing the message; `age` is seconds since it was last shown.
#[derive(Component)]
struct ToastBox {
    age: f32,
}

#[derive(Component)]
struct ToastText;

fn spawn_toast(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(48),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        children![(
            ToastBox { age: f32::MAX },
            Node {
                padding: UiRect::axes(px(18), px(8)),
                border_radius: BorderRadius::all(px(10)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            Pickable::IGNORE,
            children![(
                ToastText,
                Text::new(""),
                TextFont {
                    font_size: px(22.0).into(),
                    ..default()
                },
                TextColor(Color::NONE),
            )],
        )],
    ));
}

/// Flash a message for the events the cube emits: projection switches
/// ([`ProjectionChanged`]) and fits ([`FitStarted`], which says what it framed).
fn show_toasts(
    mut projection: MessageReader<ProjectionChanged>,
    mut fits: MessageReader<FitStarted>,
    frames: Res<ViewCubeFrames>,
    mut boxes: Query<&mut ToastBox>,
    mut texts: Query<&mut Text, With<ToastText>>,
) {
    let mut message = None;
    for p in projection.read() {
        message = Some(match p.projection {
            ViewProjection::Perspective => "Perspective".to_string(),
            ViewProjection::Orthographic => "Orthographic".to_string(),
        });
    }
    for f in fits.read() {
        message = Some(match f.source {
            FitSource::Selection => "Fit: selection".to_string(),
            FitSource::Frame(i) => format!("Fit: {}", frames.frames[i].name),
            FitSource::Scene => "Fit: whole scene".to_string(),
        });
    }
    let Some(message) = message else { return };
    if let (Ok(mut toast), Ok(mut text)) = (boxes.single_mut(), texts.single_mut()) {
        toast.age = 0.0;
        text.0 = message;
    }
}

fn fade_toast(
    time: Res<Time>,
    mut boxes: Query<(&mut ToastBox, &mut BackgroundColor)>,
    mut texts: Query<&mut TextColor, With<ToastText>>,
) {
    let (Ok((mut toast, mut background)), Ok(mut text)) = (boxes.single_mut(), texts.single_mut())
    else {
        return;
    };
    toast.age = (toast.age + time.delta_secs()).min(TOAST_HOLD + TOAST_FADE);
    let alpha = (1.0 - (toast.age - TOAST_HOLD) / TOAST_FADE).clamp(0.0, 1.0);
    background.0 = Color::srgba(0.12, 0.13, 0.16, 0.85 * alpha);
    text.0 = Color::srgba(0.95, 0.96, 0.98, alpha);
}

/// Hosts can react to frame changes, e.g. to show an OSD.
fn log_frame_changes(mut changed: MessageReader<FrameChanged>) {
    for c in changed.read() {
        info!("view cube frame -> {} (#{})", c.name, c.index);
    }
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
    mut frames: ResMut<ViewCubeFrames>,
) {
    // A little scene to orbit: a ground plane and a few boxes.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(10.0, 10.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.3, 0.3, 0.35))),
    ));
    // Three buildings, each yawed/tilted by its own frame rotation. Their
    // unequal footprints make the orientation easy to read.
    for (i, b) in buildings().into_iter().enumerate() {
        let mut building = commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(b.size.x, b.size.y, b.size.z))),
            MeshMaterial3d(materials.add(Color::srgb(0.6, 0.4 + 0.15 * i as f32, 0.3))),
            Transform::from_translation(b.position).with_rotation(b.rotation),
        ));
        // Each building is its frame's content: with "Building B" active, the
        // fit button (and picking the frame) frames just that building.
        // Frame 0 is "World", which has no content of its own.
        frames.frames[i + 1].content = Some(building.id());
        // A selection wins over frame content; here, optionally B.
        if i == 1 && std::env::var("VIEW_CUBE_FIT").is_ok_and(|v| v == "sel") {
            building.insert(FitTarget);
        }
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
        // Our toast UI renders on this camera, under the cube's overlay.
        IsDefaultUiCamera,
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
