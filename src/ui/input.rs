//! Gamepad, keyboard and mouse → one stream of navigation actions.

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    JumpBack,
    JumpForward,
    PrevBlade,
    NextBlade,
    Accept,
    Back,
    Details,
    Guide,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Device {
    Gamepad,
    #[default]
    Keyboard,
}

/// A controller was plugged in (`true`) or the last one was removed (`false`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PadEvent(pub bool);

/// This frame's actions plus repeat bookkeeping.
#[derive(Resource, Default)]
pub struct NavInput {
    pub actions: Vec<Nav>,
    pub device: Device,
    /// Set when the mouse moved this frame (hover selects tiles).
    pub mouse_moved: bool,
    pub clicked: bool,
    /// Set for one frame when a controller connects or the last one disconnects.
    pub pad_event: Option<PadEvent>,
    /// Pin the prompt style (scripted screenshots); `None` follows real input.
    pub forced: Option<Device>,
    pads: usize,
    initialised: bool,
    /// Mouse travel since the controller was last used; small bumps don't switch prompts.
    mouse_travel: f32,
    held: [f32; 4],
    next_fire: [f32; 4],
    cursor_idle: f32,
}

const REPEAT_DELAY: f32 = 0.36;

/// Directions that auto-repeat.
const DIRS: [Nav; 4] = [Nav::Up, Nav::Down, Nav::Left, Nav::Right];

fn repeat_interval(held: f32) -> f32 {
    if held > 1.4 { 0.05 } else { 0.11 }
}

#[allow(clippy::too_many_arguments)]
pub fn gather_input(
    mut nav: ResMut<NavInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    gamepads: Query<&Gamepad>,
    mut windows: Query<(&Window, &mut bevy::window::CursorOptions), With<PrimaryWindow>>,
    mut last_cursor: Local<Option<Vec2>>,
    time: Res<Time>,
) {
    nav.actions.clear();
    nav.mouse_moved = false;
    nav.clicked = false;
    nav.pad_event = None;

    // Prompts start out matching what's plugged in, then follow hot-plugging.
    let pads = gamepads.iter().count();
    if !nav.initialised {
        nav.initialised = true;
        nav.device = if pads > 0 { Device::Gamepad } else { Device::Keyboard };
    } else if pads > nav.pads {
        nav.pad_event = Some(PadEvent(true));
        nav.device = Device::Gamepad;
    } else if pads == 0 && nav.pads > 0 {
        nav.pad_event = Some(PadEvent(false));
        nav.device = Device::Keyboard;
    }
    nav.pads = pads;

    let Ok((window, mut cursor)) = windows.single_mut() else { return };
    // Never react while a game has focus.
    if !window.focused {
        nav.held = [0.0; 4];
        return;
    }
    let dt = time.delta_secs();

    let mut down = [false; 4];
    let mut kb_used = false;
    let mut pad_used = false;
    let key_any = |ks: &[KeyCode]| ks.iter().any(|k| keys.pressed(*k));
    let k = [
        key_any(&[KeyCode::ArrowUp, KeyCode::KeyW]),
        key_any(&[KeyCode::ArrowDown, KeyCode::KeyS]),
        key_any(&[KeyCode::ArrowLeft, KeyCode::KeyA]),
        key_any(&[KeyCode::ArrowRight, KeyCode::KeyD]),
    ];
    for i in 0..4 {
        if k[i] {
            down[i] = true;
            kb_used = true;
        }
    }
    for pad in &gamepads {
        let y = pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0);
        let x = pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
        let p = [
            pad.pressed(GamepadButton::DPadUp) || y > 0.6,
            pad.pressed(GamepadButton::DPadDown) || y < -0.6,
            pad.pressed(GamepadButton::DPadLeft) || x < -0.55,
            pad.pressed(GamepadButton::DPadRight) || x > 0.55,
        ];
        for i in 0..4 {
            if p[i] {
                down[i] = true;
                pad_used = true;
            }
        }
        let jp = |b| pad.just_pressed(b);
        let map = [
            (GamepadButton::South, Nav::Accept),
            (GamepadButton::East, Nav::Back),
            (GamepadButton::North, Nav::Details),
            (GamepadButton::Start, Nav::Guide),
            (GamepadButton::Select, Nav::Guide),
            (GamepadButton::Mode, Nav::Guide),
            (GamepadButton::LeftTrigger, Nav::PrevBlade),
            (GamepadButton::RightTrigger, Nav::NextBlade),
            (GamepadButton::LeftTrigger2, Nav::JumpBack),
            (GamepadButton::RightTrigger2, Nav::JumpForward),
        ];
        for (b, a) in map {
            if jp(b) {
                nav.actions.push(a);
            }
        }
        // Any button, or a deliberate push on a stick or trigger, counts as using the pad.
        let analog = [
            GamepadAxis::LeftStickX,
            GamepadAxis::LeftStickY,
            GamepadAxis::RightStickX,
            GamepadAxis::RightStickY,
        ]
        .iter()
        .any(|a| pad.get(*a).unwrap_or(0.0).abs() > 0.4);
        if pad.get_just_pressed().next().is_some() || analog {
            pad_used = true;
        }
    }

    for i in 0..4 {
        if down[i] {
            if nav.held[i] == 0.0 {
                nav.actions.push(DIRS[i]);
                nav.next_fire[i] = REPEAT_DELAY;
            } else if nav.held[i] >= nav.next_fire[i] {
                nav.actions.push(DIRS[i]);
                let held = nav.held[i];
                nav.next_fire[i] += repeat_interval(held);
            }
            nav.held[i] += dt.max(1e-4);
        } else {
            nav.held[i] = 0.0;
        }
    }

    let jk = |ks: &[KeyCode]| ks.iter().any(|k| keys.just_pressed(*k));
    let shift = key_any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if jk(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space]) {
        nav.actions.push(Nav::Accept);
    }
    if jk(&[KeyCode::Backspace]) {
        nav.actions.push(Nav::Back);
    }
    if jk(&[KeyCode::Escape, KeyCode::Home]) {
        nav.actions.push(Nav::Guide);
    }
    if jk(&[KeyCode::KeyI, KeyCode::KeyY]) {
        nav.actions.push(Nav::Details);
    }
    if jk(&[KeyCode::KeyQ]) || (shift && jk(&[KeyCode::Tab])) {
        nav.actions.push(Nav::PrevBlade);
    }
    if jk(&[KeyCode::KeyE]) || (!shift && jk(&[KeyCode::Tab])) {
        nav.actions.push(Nav::NextBlade);
    }
    if jk(&[KeyCode::PageUp]) {
        nav.actions.push(Nav::JumpBack);
    }
    if jk(&[KeyCode::PageDown]) {
        nav.actions.push(Nav::JumpForward);
    }
    if keys.get_just_pressed().next().is_some() {
        kb_used = true;
    }

    // Mouse.
    let pos = window.cursor_position();
    let mut mouse_used = false;
    if pos.is_some() && pos != *last_cursor {
        if let (Some(a), Some(b)) = (pos, *last_cursor) {
            nav.mouse_moved = true;
            nav.mouse_travel += a.distance(b);
        }
        *last_cursor = pos;
    }
    nav.mouse_travel *= (-dt * 1.5).exp();
    if nav.mouse_travel > 24.0 {
        mouse_used = true;
    }
    if mouse.just_pressed(MouseButton::Left) {
        nav.clicked = true;
        nav.mouse_moved = true;
        mouse_used = true;
    }
    if mouse.just_pressed(MouseButton::Right) {
        nav.actions.push(Nav::Back);
    }
    let wheel = scroll.delta.y + scroll.delta.x;
    if wheel > 0.0 {
        nav.actions.push(Nav::Left);
        mouse_used = true;
    } else if wheel < 0.0 {
        nav.actions.push(Nav::Right);
        mouse_used = true;
    }

    if pad_used {
        nav.device = Device::Gamepad;
        nav.mouse_travel = 0.0;
    } else if kb_used || mouse_used {
        nav.device = Device::Keyboard;
    }
    if let Some(d) = nav.forced {
        nav.device = d;
    }

    // Hide the cursor after a few idle seconds or when a pad is in use.
    if nav.mouse_moved {
        nav.cursor_idle = 0.0;
    } else {
        nav.cursor_idle += dt;
    }
    let visible = nav.cursor_idle < 3.0 && nav.device == Device::Keyboard;
    if cursor.visible != visible {
        cursor.visible = visible;
    }
}
