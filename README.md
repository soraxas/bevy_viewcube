# bevy_viewcube

An AutoCAD-style **view cube** for Bevy: a small 3D orientation gizmo pinned to
a corner of the screen. It mirrors a camera's orientation; click or drag it to
snap or orbit the camera.

- Clickable faces, edges and corners snap to the matching view and keep the
  screen-up axis sensible; drag to orbit.
- Home button (iso view), zoom-to-fit button, optional coordinate-frame dropdown.
- Double-click the home button to toggle perspective / orthographic.
- Camera-agnostic: works with a plain `Camera3d`, or `bevy_editor_cam`.
- Text is rasterised from Bevy's own `Font` (via `swash`, already used by
  `bevy_text`), so there is no extra font crate.

## Usage

```rust
use bevy::prelude::*;
use bevy_viewcube::{driver::BasicDriverPlugin, ViewCubePlugin, ViewCubeTarget};

App::new()
    .add_plugins((DefaultPlugins, ViewCubePlugin::default(), BasicDriverPlugin::default()))
    .add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Camera3d::default(), ViewCubeTarget));
    })
    .run();
```

With `bevy_editor_cam`, use `EditorCamDriverPlugin::default()` instead and put
`ViewCubeTarget` on your `EditorCam` camera.

Keep the `ViewCubeConfig` resource current: `active`, and the rectangle
(`origin` + `panel_size`, physical px) and `corner` the cube pins to.

Cube text uses Bevy's built-in default font, so no asset files are needed. To
use your own, set `ViewCubeSettings::font` to an asset path (TrueType and
OpenType both work).

## Design

The cube never calls a camera controller. It reads the target's `Transform` /
`Projection`, decides what should happen and emits a `CameraRequest` message; a
**driver** applies it.

| Driver | Needs | Use when |
|---|---|---|
| `driver::BasicDriverPlugin` | nothing | a plain `Camera3d` |
| `driver::EditorCamDriverPlugin` (feature `editor_cam`) | `bevy_editor_cam` | you use `EditorCam` |
| your own | — | read `CameraRequest` after `ViewCubeSet` |

`bevy_editor_cam` is touched only in `src/driver/editor_cam.rs`. Look-to and fit
animations are the crate's own (`driver::TweenPlugin`).

The cube renders on its own render layer through an overlay camera, so it never
touches your scene.

**Settings vs config:** options fixed at build time (style, render layer, font,
axis letters, face captions, axes placement) are `ViewCubeSettings` on the
plugin. Runtime state is the `ViewCubeConfig` resource and `ViewCubeFrames`.

## Features

- `ui` (default): home / fit buttons and frame dropdown as `bsn!` Bevy UI. They
  only send messages (`SnapView`, `FitView`, `SetFrame`), so without the feature
  you can drive the cube from your own UI by writing the same messages. Nodes
  are placed in logical px, assuming the primary window.
- `editor_cam` (default): the `EditorCamDriverPlugin` adapter.

## Behaviour

**Views and up axis.** A click on a face, edge or corner snaps to that view. The
screen-up axis is the frame axis (flattened into the view plane) closest to the
camera's current up, i.e. the smallest roll; if all are orthogonal, the frame's
own up is used (+Y, or -Z for TOP). Double-clicking a face rolls to the opposite
up axis (`FlipViewUp`).

**Projection.** Double-clicking home toggles perspective / orthographic (a
dolly-zoom, so the focused object keeps its size). A single click snaps to iso
after `double_click_delay`. Send `ToggleProjection` yourself for a hotkey; read
`ProjectionChanged` for an OSD. `ViewCubeConfig::double_click_projection = false`
disables the binding.

**Zoom to fit.** The fit button sends `FitView` (send it for a hotkey). It frames,
in priority order: entities tagged `FitTarget`; the active frame's `content`
(`ViewCubeFrame::with_content(entity)`); every entity with an `Aabb`. Entities
with an `Aabb` are fit by their projected corners; those with only a translation
are centered at the current distance. `FitStarted { source }` fires when a fit
begins. `ViewCubeConfig::fit_padding` sets the margin and `fit_on_frame_change`
also fits when the active frame changes.

**Coordinate frames.** Give the plugin `ViewCubeFrames` and a dropdown appears
above the cube. Every snap resolves in the active frame, so FRONT in a
"Building" frame faces the building's front. Switch with `SetFrame(index)`;
`FrameChanged` fires on any change. Drivers orbit about the frame's up so the
roll survives orbiting. The host may edit the list at any time. With no frames
there is no dropdown and the cube is world-aligned.

```rust
ViewCubePlugin {
    frames: ViewCubeFrames {
        frames: vec![
            ViewCubeFrame::new("World", Quat::IDENTITY),
            ViewCubeFrame::new("Building", Quat::from_rotation_y(0.52)),
        ],
        ..default()
    },
    ..default()
}
```

The active frame's X / Y / Z axes are drawn as a small triad
(`ViewCubeSettings::axes`): `AxesPlacement::ViewportCorner(corner)` (default),
`CubeCorner` or `Hidden`.

**Styles.** `ViewCubeStyle::Cad` (default) is the solid captioned cube with
hover highlighting; `ViewCubeStyle::Axes` is an open Blender-style axis gizmo.

## Examples

`cargo run --example <name>`: `basic` (CAD cube), `axes` (axis gizmo), `frames`
(coordinate frames, fit, toasts), `builtin` (no `bevy_editor_cam`).

### In the browser (touch)

Any example builds for wasm; `examples/wasm/serve.sh [example]` (default
`frames`) builds one and serves it on your LAN, to try touch on a phone:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>
examples/wasm/serve.sh        # then open http://<your-ip>:8080 on the device
```

Touches arrive as primary-button pointers, so tap, double-tap and drag on the
cube behave as with a mouse. (`bevy_editor_cam`'s own viewport controls are
mouse-only.) The build uses the size-optimised `web` profile; `PROFILE=dev` is
several hundred MB.
