//! The cube's buttons as Bevy UI, authored with `bsn!`: a **home** button
//! (snaps to the iso view) and a **frame dropdown** (picks the active
//! [`ViewCubeFrames`] frame). They only ever *send messages* — [`SnapView`] and
//! [`SetFrame`] — which the cube's own systems act on, so a host with a
//! different UI toolkit can drive the cube by writing the same messages.
//!
//! The nodes live under a window-sized, pointer-transparent root with a child
//! positioned over the cube viewport. They render through a dedicated overlay
//! UI camera ordered *above* the cube camera: otherwise the cube's pixels would
//! be drawn over the nodes, and the cube would win picking where they overlap
//! (the dropdown list sits over the cube).

use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::text::FontSourceTemplate;
use bevy::camera::visibility::RenderLayers;

use crate::systems::cube_viewport_rect;
use crate::{
    CubeSettings, FitView, SetFrame, SnapView, ToggleProjection, ViewCubeConfig, ViewCubeFrames,
    ViewCubeStyle, ViewPreset,
};

/// Order of the overlay UI camera: above the cube camera (10).
const UI_CAMERA_ORDER: isize = 20;

const BUTTON_BG: Color = Color::srgba(0.20, 0.22, 0.26, 0.92);
const BUTTON_HOVER: Color = Color::srgba(0.30, 0.34, 0.42, 0.96);
const POPUP_BG: Color = Color::srgba(0.16, 0.18, 0.21, 0.98);
const ITEM_ACTIVE: Color = Color::srgba(0.30, 0.58, 0.95, 0.85);
const ICON: Color = Color::srgb(0.80, 0.83, 0.88);
const TEXT: Color = Color::srgb(0.92, 0.93, 0.95);
const TEXT_DIM: Color = Color::srgb(0.65, 0.68, 0.74);

/// Window-sized root of the cube's UI; hidden while the cube is inactive.
#[derive(Component, Default, Clone)]
struct CubeUiRoot;

/// Child of the root, kept over the cube viewport rect.
#[derive(Component, Default, Clone)]
struct CubeUiRect;

/// Wrapper of the frame button; hidden when there are no frames.
#[derive(Component, Default, Clone)]
struct CubeFrameDropdown;

/// The frame button itself (the popup is its child).
#[derive(Component, Default, Clone)]
struct CubeFrameButton;

/// The text showing the active frame's name.
#[derive(Component, Default, Clone)]
struct CubeFrameName;

/// The open list of frames.
#[derive(Component, Default, Clone)]
struct CubeFramePopup;

/// A home-button click waiting out the double-click window before it snaps.
#[derive(Resource, Default)]
struct PendingHomeSnap {
    /// `Time::elapsed_secs_f64` at which to snap, if no second click comes.
    due: Option<f64>,
}

pub(crate) fn build(app: &mut App) {
    app.init_resource::<PendingHomeSnap>()
        .add_systems(Startup, setup_ui)
        .add_systems(
            Update,
            (
                sync_ui_rect,
                sync_dropdown,
                close_popup_on_outside_press,
                fire_pending_home_snap,
            ),
        );
}

/// Snap to iso once a home-button click's double-click window has passed.
fn fire_pending_home_snap(
    time: Res<Time>,
    mut pending: ResMut<PendingHomeSnap>,
    mut snap: MessageWriter<SnapView>,
) {
    if pending.due.is_some_and(|due| time.elapsed_secs_f64() >= due) {
        pending.due = None;
        snap.write(SnapView(ViewPreset::Iso));
    }
}

fn setup_ui(
    mut commands: Commands,
    settings: Res<CubeSettings>,
    frames: Res<ViewCubeFrames>,
) {
    if settings.0.style != ViewCubeStyle::Cad {
        return;
    }
    // Renders nothing but our UI: no layers, so host scene content on the
    // default layer isn't drawn a second time.
    let camera = commands
        .spawn((
            Camera2d,
            Camera {
                order: UI_CAMERA_ORDER,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            RenderLayers::none(),
            Name::new("ViewCubeUiCamera"),
        ))
        .id();
    let name = frames
        .frames
        .get(frames.active)
        .map(|f| f.name.clone())
        .unwrap_or_default();
    commands
        .queue_spawn_scene(ui_root(settings.0.font.clone(), settings.0.dropdown_caret.clone(), name))
        .insert((UiTargetCamera(camera), GlobalZIndex(1000)));
}

/// The configured font for UI text: the asset at `font`, else Bevy's default.
fn font_source(font: Option<String>) -> FontSourceTemplate {
    match font {
        Some(path) => FontSourceTemplate::Handle(path.into()),
        None => FontSourceTemplate::default(),
    }
}

fn ui_root(font: Option<String>, caret: String, name: String) -> impl Scene {
    bsn! {
        CubeUiRoot
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
        }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [(
            CubeUiRect
            Node { position_type: PositionType::Absolute }
            Pickable { should_block_lower: false, is_hoverable: false }
            Children [
                ({home_button()}),
                ({fit_button()}),
                ({frame_dropdown(font, caret, name)}),
            ]
        )]
    }
}

/// A small house: a rotated square roof over a square body.
fn home_button() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(2),
            top: px(2),
            width: px(24),
            height: px(24),
            border_radius: BorderRadius::all(px(6)),
        }
        BackgroundColor(BUTTON_BG)
        on(|
            click: On<Pointer<Click>>,
            config: Res<ViewCubeConfig>,
            time: Res<Time>,
            mut pending: ResMut<PendingHomeSnap>,
            mut snap: MessageWriter<SnapView>,
            mut toggle: MessageWriter<ToggleProjection>,
        | {
            // Bevy fires every click immediately and only tags it with a count,
            // so a double-click's first click can't be told apart from a
            // single click. Hold the iso snap for a moment: a second click
            // cancels it and toggles the projection instead.
            let defer = config.double_click_projection && !config.double_click_delay.is_zero();
            if click.button != PointerButton::Primary {
                return;
            }
            match click.count {
                1 if defer => {
                    pending.due = Some(time.elapsed_secs_f64() + config.double_click_delay.as_secs_f64());
                }
                1 => {
                    snap.write(SnapView(ViewPreset::Iso));
                }
                2 if config.double_click_projection => {
                    pending.due = None;
                    toggle.write(ToggleProjection);
                }
                _ => {}
            }
        })
        on(highlight_on_over)
        on(restore_on_out)
        Children [
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: px(7),
                    top: px(5),
                    width: px(10),
                    height: px(10),
                }
                UiTransform { rotation: Rot2::degrees(45.0) }
                BackgroundColor(ICON)
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: px(6),
                    top: px(11),
                    width: px(12),
                    height: px(9),
                }
                BackgroundColor(ICON)
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
        ]
    }
}

/// "Zoom to fit": a frame outline with a dot inside it.
fn fit_button() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(2),
            top: px(30),
            width: px(24),
            height: px(24),
            border_radius: BorderRadius::all(px(6)),
        }
        BackgroundColor(BUTTON_BG)
        on(|click: On<Pointer<Click>>, mut fit: MessageWriter<FitView>| {
            if click.button == PointerButton::Primary {
                fit.write(FitView);
            }
        })
        on(highlight_on_over)
        on(restore_on_out)
        Children [
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: px(6),
                    top: px(6),
                    width: px(12),
                    height: px(12),
                    border: UiRect::all(px(2)),
                }
                BorderColor::from(ICON)
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: px(10),
                    top: px(10),
                    width: px(4),
                    height: px(4),
                }
                BackgroundColor(ICON)
                Pickable { should_block_lower: false, is_hoverable: false }
            ),
        ]
    }
}

fn frame_dropdown(font: Option<String>, caret: String, name: String) -> impl Scene {
    let caret_font = font.clone();
    bsn! {
        CubeFrameDropdown
        Node {
            position_type: PositionType::Absolute,
            // Right of the home button, centered in what's left.
            left: px(28),
            right: px(2),
            top: px(3),
            justify_content: JustifyContent::Center,
            display: Display::None,
        }
        Pickable { should_block_lower: false, is_hoverable: false }
        Children [(
            CubeFrameButton
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(4),
                padding: UiRect::axes(px(7), px(3)),
                border_radius: BorderRadius::all(px(6)),
            }
            BackgroundColor(BUTTON_BG)
            on(toggle_popup)
            on(highlight_on_over)
            on(restore_on_out)
            Children [
                (
                    CubeFrameName
                    Text({name})
                    TextFont { font: {font_source(font)}, font_size: px(10.0) }
                    TextColor(TEXT)
                    Pickable { should_block_lower: false, is_hoverable: false }
                ),
                (
                    Text({caret})
                    TextFont { font: {font_source(caret_font)}, font_size: px(9.0) }
                    TextColor(TEXT_DIM)
                    Pickable { should_block_lower: false, is_hoverable: false }
                ),
            ]
        )]
    }
}

fn frame_popup(font: Option<String>, names: Vec<String>, active: usize) -> impl Scene {
    let items: Vec<_> = names
        .into_iter()
        .enumerate()
        .map(|(i, name)| frame_item(font.clone(), name, i, i == active))
        .collect();
    bsn! {
        CubeFramePopup
        Node {
            position_type: PositionType::Absolute,
            top: percent(100),
            left: px(0),
            min_width: percent(100),
            margin: UiRect::top(px(4)),
            padding: UiRect::all(px(3)),
            flex_direction: FlexDirection::Column,
            border_radius: BorderRadius::all(px(8)),
        }
        BackgroundColor(POPUP_BG)
        Children [{items}]
    }
}

fn frame_item(font: Option<String>, name: String, index: usize, active: bool) -> impl Scene {
    let resting = if active { ITEM_ACTIVE } else { Color::NONE };
    bsn! {
        Node {
            padding: UiRect::axes(px(8), px(3)),
            border_radius: BorderRadius::all(px(5)),
        }
        BackgroundColor({resting})
        on(move |
            mut click: On<Pointer<Click>>,
            mut set: MessageWriter<SetFrame>,
            mut commands: Commands,
            popup: Query<Entity, With<CubeFramePopup>>,
        | {
            // Don't let the click reach the button (it would reopen the popup).
            click.propagate(false);
            if click.button != PointerButton::Primary {
                return;
            }
            set.write(SetFrame(index));
            for p in &popup {
                commands.entity(p).despawn();
            }
        })
        on(|over: On<Pointer<Over>>, mut q: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = q.get_mut(over.event_target()) {
                bg.0 = BUTTON_HOVER;
            }
        })
        on(move |out: On<Pointer<Out>>, mut q: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = q.get_mut(out.event_target()) {
                bg.0 = resting;
            }
        })
        Children [(
            Text({name})
            TextFont { font: {font_source(font)}, font_size: px(10.0) }
            TextColor(TEXT)
            Pickable { should_block_lower: false, is_hoverable: false }
        )]
    }
}

fn highlight_on_over(over: On<Pointer<Over>>, mut q: Query<&mut BackgroundColor>) {
    if let Ok(mut bg) = q.get_mut(over.event_target()) {
        bg.0 = BUTTON_HOVER;
    }
}

fn restore_on_out(out: On<Pointer<Out>>, mut q: Query<&mut BackgroundColor>) {
    if let Ok(mut bg) = q.get_mut(out.event_target()) {
        bg.0 = BUTTON_BG;
    }
}

/// Open the frame list under the button, or close it if already open.
fn toggle_popup(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    popup: Query<Entity, With<CubeFramePopup>>,
    frames: Res<ViewCubeFrames>,
    settings: Res<CubeSettings>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    if let Ok(open) = popup.single() {
        commands.entity(open).despawn();
        return;
    }
    if frames.frames.is_empty() {
        return;
    }
    let names = frames.frames.iter().map(|f| f.name.clone()).collect();
    commands
        .queue_spawn_scene(frame_popup(settings.0.font.clone(), names, frames.active))
        .insert(ChildOf(click.event_target()));
}

/// Keep the UI over the cube viewport (physical px → logical px) and hide it
/// while the cube is inactive.
fn sync_ui_rect(
    config: Res<ViewCubeConfig>,
    windows: Query<&Window>,
    mut root: Query<&mut Node, (With<CubeUiRoot>, Without<CubeUiRect>)>,
    mut rect: Query<&mut Node, (With<CubeUiRect>, Without<CubeUiRoot>)>,
) {
    let scale = windows
        .iter()
        .next()
        .map(|w| w.scale_factor())
        .unwrap_or(1.0);
    let display = if config.active {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut root {
        if node.display != display {
            node.display = display;
        }
    }
    let Some((position, size)) = cube_viewport_rect(&config, scale) else {
        return;
    };
    let (left, top, side) = (
        px(position.x as f32 / scale),
        px(position.y as f32 / scale),
        px(size.x as f32 / scale),
    );
    for mut node in &mut rect {
        if node.left != left || node.top != top || node.width != side || node.height != side {
            node.left = left;
            node.top = top;
            node.width = side;
            node.height = side;
        }
    }
}

/// Show the dropdown only when there are frames, keep its label on the active
/// frame, and drop a stale open list when the frames change underneath it.
fn sync_dropdown(
    frames: Res<ViewCubeFrames>,
    mut commands: Commands,
    mut dropdown: Query<&mut Node, With<CubeFrameDropdown>>,
    mut name: Query<&mut Text, With<CubeFrameName>>,
    popup: Query<Entity, With<CubeFramePopup>>,
) {
    // Not gated on `is_changed`: the scene is spawned from a queue, so the nodes
    // may not exist yet on the frame the frames first appear.
    let display = if frames.frames.is_empty() {
        Display::None
    } else {
        Display::Flex
    };
    for mut node in &mut dropdown {
        if node.display != display {
            node.display = display;
        }
    }
    if let Some(frame) = frames.frames.get(frames.active) {
        for mut text in &mut name {
            if text.0 != frame.name {
                text.0 = frame.name.clone();
            }
        }
    }
    if frames.is_changed() {
        for p in &popup {
            commands.entity(p).despawn();
        }
    }
}

/// Close the open list on Escape or on a press anywhere outside the button.
fn close_popup_on_outside_press(
    mut commands: Commands,
    popup: Query<Entity, With<CubeFramePopup>>,
    button: Query<Entity, With<CubeFrameButton>>,
    parents: Query<&ChildOf>,
    hover: Res<HoverMap>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let Ok(open) = popup.single() else { return };
    let escape = keys.just_pressed(KeyCode::Escape);
    let pressed = mouse.just_pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Right);
    if !escape && !pressed {
        return;
    }
    let inside = button.single().is_ok_and(|button| {
        hover.get(&PointerId::Mouse).is_some_and(|hits| {
            hits.keys().any(|&hit| {
                hit == button || parents.iter_ancestors(hit).any(|a| a == button)
            })
        })
    });
    if escape || !inside {
        commands.entity(open).despawn();
    }
}
