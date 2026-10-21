//! benilla: a mouse on Android as a mouse. Upstream turns every motion event into a touch, so a
//! mouse only ever reported a finger: no hover, one button, no wheel. Here a mouse's events are
//! the desktop's: `CursorMoved` with and without a button down, `MouseInput` for each button
//! (`ButtonPress`/`ButtonRelease`, which carry which one), `MouseWheel` from the scroll axes,
//! `CursorEntered`/`CursorLeft` from hover enter and exit, and `DeviceEvent::MouseMotion`, from
//! the captured pointer's own deltas or else the cursor's step.

use android_activity::input::{Axis, Button, MotionAction, MotionEvent};

use super::{DeviceId, WindowId};
use crate::dpi::PhysicalPosition;
use crate::event::{
    DeviceEvent, ElementState, Event, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use crate::window;

/// `AINPUT_SOURCE_MOUSE`: the pointer class plus the mouse bit.
const MOUSE: u32 = 0x0000_2002;
/// `AINPUT_SOURCE_MOUSE_RELATIVE`: a captured pointer, reporting deltas.
const MOUSE_RELATIVE: u32 = 0x0002_0004;
/// `AINPUT_SOURCE_GAMEPAD` and `AINPUT_SOURCE_JOYSTICK`'s own bits, without their classes.
const GAMEPAD_BIT: u32 = 0x0000_0400;
const JOYSTICK_BIT: u32 = 0x0100_0000;

/// Whether `source` is a mouse, captured or not.
pub(super) fn is_mouse(source: u32) -> bool {
    source & MOUSE == MOUSE || source & MOUSE_RELATIVE == MOUSE_RELATIVE
}

/// Whether `source` is a game controller: its sticks, triggers and buttons. A keyboard's arrows
/// share the D-pad's key codes but carry neither bit.
pub(super) fn is_controller(source: u32) -> bool {
    source & (GAMEPAD_BIT | JOYSTICK_BIT) != 0
}

#[derive(Debug, Default)]
pub(super) struct Mouse {
    /// The cursor's last position, for `MouseMotion` while the pointer is not captured.
    last: Option<(f32, f32)>,
    /// A `Down` with no button behind it (a touchpad's tap) pressed the left button.
    tapped: bool,
    /// The finger driving the cursor ([`Self::finger`]), by pointer id; others are ignored.
    finger: Option<i32>,
}

impl Mouse {
    /// benilla: a finger on the screen as the mouse's left button. Down presses it where the
    /// finger is, a move drags the cursor (and its `MouseMotion`), and lifting releases it with
    /// the cursor left where the finger lifted, as a mouse is: an app's button that clicks on a
    /// release over it then clicks on a tap, which a touch, gone once lifted, never gave. Only
    /// the first finger down drives it; a second is ignored. No touch events go out.
    pub(super) fn finger<T: 'static>(&mut self, ev: &MotionEvent<'_>, emit: &mut impl FnMut(Event<T>)) {
        let device_id = crate::event::DeviceId(DeviceId(ev.device_id()));
        let window_id = window::WindowId(WindowId);
        let win = |event| Event::WindowEvent::<T> { window_id, event };
        let button = |state| WindowEvent::MouseInput {
            device_id,
            state,
            button: MouseButton::Left,
        };
        let action = ev.action();
        let pointer = ev.pointer_at_index(ev.pointer_index());
        let id = pointer.pointer_id();
        match action {
            MotionAction::Down if self.finger.is_none() => {
                self.finger = Some(id);
                let (x, y) = (pointer.x(), pointer.y());
                self.last = None;
                self.moved(x, y, device_id, emit);
                emit(win(button(ElementState::Pressed)));
            },
            MotionAction::Move => {
                let Some(held) = self.finger else { return };
                if let Some(p) = ev.pointers().find(|p| p.pointer_id() == held) {
                    self.moved(p.x(), p.y(), device_id, emit);
                }
            },
            MotionAction::Up | MotionAction::PointerUp | MotionAction::Cancel
                if self.finger == Some(id) || action == MotionAction::Cancel =>
            {
                if self.finger.take().is_some() {
                    self.moved(pointer.x(), pointer.y(), device_id, emit);
                    emit(win(button(ElementState::Released)));
                }
            },
            _ => {},
        }
    }

    /// The cursor to `(x, y)`, and the step as `MouseMotion` once there was a last position.
    fn moved<T: 'static>(&mut self, x: f32, y: f32, device_id: crate::event::DeviceId, emit: &mut impl FnMut(Event<T>)) {
        if self.last == Some((x, y)) {
            return;
        }
        emit(Event::WindowEvent {
            window_id: window::WindowId(WindowId),
            event: WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(x as f64, y as f64),
            },
        });
        if let Some((lx, ly)) = self.last {
            emit(Event::DeviceEvent {
                device_id,
                event: DeviceEvent::MouseMotion { delta: ((x - lx) as f64, (y - ly) as f64) },
            });
        }
        self.last = Some((x, y));
    }

    pub(super) fn motion<T: 'static>(
        &mut self,
        ev: &MotionEvent<'_>,
        source: u32,
        emit: &mut impl FnMut(Event<T>),
    ) {
        let device_id = crate::event::DeviceId(DeviceId(ev.device_id()));
        let window_id = window::WindowId(WindowId);
        let win = |event| Event::WindowEvent::<T> { window_id, event };
        let action = ev.action();
        let pointer = ev.pointer_at_index(0);
        let (x, y) = (pointer.x(), pointer.y());

        if source & MOUSE_RELATIVE == MOUSE_RELATIVE && source & MOUSE != MOUSE {
            // Captured: the axes are the motion itself, and there is no cursor.
            if x != 0.0 || y != 0.0 {
                emit(Event::DeviceEvent {
                    device_id,
                    event: DeviceEvent::MouseMotion { delta: (x as f64, y as f64) },
                });
            }
        } else if action != MotionAction::Scroll {
            match action {
                MotionAction::HoverEnter => emit(win(WindowEvent::CursorEntered { device_id })),
                MotionAction::HoverExit => {
                    self.last = None;
                    emit(win(WindowEvent::CursorLeft { device_id }));
                    return;
                },
                _ => {},
            }
            if self.last != Some((x, y)) {
                emit(win(WindowEvent::CursorMoved {
                    device_id,
                    position: PhysicalPosition::new(x as f64, y as f64),
                }));
                if let Some((lx, ly)) = self.last {
                    emit(Event::DeviceEvent {
                        device_id,
                        event: DeviceEvent::MouseMotion {
                            delta: ((x - lx) as f64, (y - ly) as f64),
                        },
                    });
                }
                self.last = Some((x, y));
            }
        }

        let win = |event| Event::WindowEvent::<T> { window_id, event };
        let button = |state, button| WindowEvent::MouseInput { device_id, state, button };
        match action {
            MotionAction::ButtonPress | MotionAction::ButtonRelease => {
                let state = if action == MotionAction::ButtonPress {
                    ElementState::Pressed
                } else {
                    ElementState::Released
                };
                let which = match ev.action_button() {
                    Button::Primary => MouseButton::Left,
                    Button::Secondary => MouseButton::Right,
                    Button::Tertiary => MouseButton::Middle,
                    Button::Back => MouseButton::Back,
                    Button::Forward => MouseButton::Forward,
                    other => MouseButton::Other(u32::from(other) as u16),
                };
                emit(win(button(state, which)));
            },
            MotionAction::Down if ev.button_state().0 == 0 => {
                self.tapped = true;
                emit(win(button(ElementState::Pressed, MouseButton::Left)));
            },
            MotionAction::Up if self.tapped => {
                self.tapped = false;
                emit(win(button(ElementState::Released, MouseButton::Left)));
            },
            MotionAction::Scroll => {
                let (h, v) = (pointer.axis_value(Axis::Hscroll), pointer.axis_value(Axis::Vscroll));
                if h != 0.0 || v != 0.0 {
                    emit(win(WindowEvent::MouseWheel {
                        device_id,
                        delta: MouseScrollDelta::LineDelta(h, v),
                        phase: TouchPhase::Moved,
                    }));
                }
            },
            _ => {},
        }
    }
}
