//! The pointer side of drag. It feeds window input to the drag machine in `drag.rs`, starts the
//! long-press timer, draws the lifted card, and routes every exit through [`WorkspaceView::end_drag`].
//!
//! Touch arrives here as mouse input. gpui 0.2.2 on Windows has no touch or pointer API, and
//! Windows sends touch as mouse messages, so the two can't be told apart in this layer.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{prelude::*, *};
use looji_domain::{Status, TaskId};

use super::WorkspaceView;
use super::board::card_view;
use super::card::{CalendarDate, CardDensity, task_card};
use super::drag::{
    self as drag, CancelReason, CardSlot, ColumnSlot, DragMachine, DropResult, Effect, Input,
    resolve_drop,
};
use crate::theme::Theme;

/// What the last paint measured, in window coordinates. Drop targets are resolved from this.
#[derive(Default)]
pub(super) struct BoardLayout {
    columns: Vec<ColumnSlot>,
    cards: Vec<CardSlot>,
}

impl BoardLayout {
    fn card(&self, task: TaskId) -> Option<&CardSlot> {
        self.cards.iter().find(|card| card.task == task)
    }

    pub(super) fn set_columns(&mut self, columns: Vec<ColumnSlot>) {
        self.columns = columns;
    }

    /// Replaces the cards of one column. Other columns keep their cards from the same paint.
    pub(super) fn set_cards(&mut self, status: Status, cards: Vec<CardSlot>) {
        self.cards.retain(|card| card.status != status);
        self.cards.extend(cards);
    }
}

/// How a drag ended. Kept after the drag so the debug controls can show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(debug_assertions), allow(dead_code))] // Read by the debug controls only.
pub(super) enum Exit {
    Dropped(DropResult),
    Cancelled(CancelReason),
}

/// The drag state owned by the workspace view.
#[derive(Default)]
pub(super) struct DragSession {
    machine: DragMachine,
    /// The long-press timer. Dropping it cancels the timer.
    timer: Option<Task<()>>,
    /// Ends the drag if the window stops being active. Dropped when the drag ends.
    activation: Option<Subscription>,
    /// Width of the lifted card, taken from the slot it came from.
    overlay_width: Option<Pixels>,
    /// Filled by the board's paint. Shared with the paint callbacks.
    pub(super) layout: Rc<RefCell<BoardLayout>>,
    #[cfg_attr(not(debug_assertions), allow(dead_code))] // Read by the debug controls only.
    last_exit: Option<Exit>,
}

impl DragSession {
    /// The card being dragged, once it has lifted. The board dims its slot.
    pub(super) fn dragged(&self) -> Option<TaskId> {
        self.machine.lifted_at().map(|(card, _)| card)
    }

    #[cfg_attr(not(debug_assertions), allow(dead_code))] // Read by the debug controls only.
    pub(super) fn last_exit(&self) -> Option<Exit> {
        self.last_exit
    }
}

/// Monotonic time for the drag machine. Only differences between calls matter.
fn now() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

fn point(position: Point<Pixels>) -> drag::Point {
    drag::Point {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

/// A painted rectangle, in the drag module's terms.
pub(super) fn rect(bounds: Bounds<Pixels>) -> drag::Rect {
    drag::Rect {
        origin: drag::Point {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
        },
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

/// Whether a point is inside the window. Mouse moves and releases still arrive outside it
/// while the button is held, so this is how leaving the window is detected.
fn inside(window: &Window, point: drag::Point) -> bool {
    let size = window.viewport_size();
    point.x >= 0.0
        && point.y >= 0.0
        && point.x < f32::from(size.width)
        && point.y < f32::from(size.height)
}

/// The registration for the window-level listeners. Runs in paint, so the listeners see every
/// mouse move and release, including those outside the board. Listeners ignore input while no
/// drag is running.
pub(super) fn listeners(view: WeakEntity<WorkspaceView>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let moved = view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = moved.update(cx, |this, cx| this.pointer_moved(event, window, cx));
                }
            });

            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = view.update(cx, |this, cx| this.pointer_released(event, window, cx));
                }
            });
        },
    )
    .absolute()
    .size_full()
}

/// The lifted card, drawn above the board at the pointer minus the grab point. It has no
/// handlers, so it doesn't take hits from the board.
pub(super) fn overlay(view: &WorkspaceView, theme: &Theme) -> Option<AnyElement> {
    let (card, top_left) = view.drag.machine.lifted_at()?;
    let width = view.drag.overlay_width?;
    let task = view.board.iter().find(|task| task.id() == card)?;
    let today = CalendarDate::utc_today();
    Some(
        div()
            .absolute()
            .left(px(top_left.x))
            .top(px(top_left.y))
            .w(width)
            .rounded(theme.radius.sm)
            .shadow(theme.elevation.overlay.clone())
            .child(task_card(
                card_view(task),
                CardDensity::Compact,
                today,
                theme,
            ))
            .into_any_element(),
    )
}

impl WorkspaceView {
    /// A press on a card. The card is either clicked, lifted by a long press or flick, or left
    /// alone. The window listeners see the rest.
    pub(super) fn card_pressed(
        &mut self,
        task: TaskId,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let origin = match self.drag.layout.borrow().card(task) {
            Some(slot) => slot.bounds.origin,
            None => return,
        };
        self.drag.last_exit = None;
        // A window that loses focus, for example on Alt-Tab, takes the release with it.
        self.drag.activation = Some(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.drive_drag(Input::Cancel(CancelReason::FocusLost), cx);
            }
        }));
        self.drive_drag(
            Input::Down {
                card: task,
                pointer: point(event.position),
                card_origin: origin,
                at: now(),
            },
            cx,
        );
    }

    fn pointer_moved(&mut self, event: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        if self.drag.machine.is_idle() {
            return;
        }
        // Without the button held, the release was lost, so the drag can't finish normally.
        if event.pressed_button != Some(MouseButton::Left) {
            return self.drive_drag(Input::Cancel(CancelReason::CaptureLost), cx);
        }
        let pointer = point(event.position);
        if !inside(window, pointer) {
            return self.drive_drag(Input::Cancel(CancelReason::PointerLeftWindow), cx);
        }
        self.drive_drag(Input::Move { pointer, at: now() }, cx);
    }

    fn pointer_released(&mut self, event: &MouseUpEvent, window: &Window, cx: &mut Context<Self>) {
        if self.drag.machine.is_idle() || event.button != MouseButton::Left {
            return;
        }
        let pointer = point(event.position);
        let input = if inside(window, pointer) {
            Input::Up { pointer }
        } else {
            Input::Cancel(CancelReason::ReleasedOutside)
        };
        self.drive_drag(input, cx);
    }

    /// Escape cancels a drag. Returns whether the key was used. Keys go through the focused
    /// element, so this is called from the workspace's key handler rather than a window listener.
    pub(super) fn key_pressed(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.drag.machine.is_idle() || event.keystroke.key != "escape" {
            return false;
        }
        self.drive_drag(Input::Cancel(CancelReason::Escape), cx);
        true
    }

    /// Feeds one input to the machine and carries out what it asks for.
    fn drive_drag(&mut self, input: Input, cx: &mut Context<Self>) {
        for effect in self.drag.machine.feed(input) {
            self.apply_effect(effect, cx);
        }
    }

    fn apply_effect(&mut self, effect: Effect, cx: &mut Context<Self>) {
        match effect {
            Effect::ArmTimer { generation, after } => {
                self.drag.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(after).await;
                    let _ = this.update(cx, |view, cx| {
                        view.drive_drag(Input::Tick { generation }, cx)
                    });
                }));
            }
            Effect::OpenCard(task) => open_card(task),
            Effect::Lift { card, .. } => {
                self.drag.overlay_width = self
                    .drag
                    .layout
                    .borrow()
                    .card(card)
                    .map(|slot| px(slot.bounds.width));
                cx.notify();
            }
            Effect::Follow { .. } => cx.notify(),
            Effect::Drop { card, pointer } => {
                let target = {
                    let layout = self.drag.layout.borrow();
                    resolve_drop(card, pointer, &layout.columns, &layout.cards)
                };
                let exit = match target {
                    Some(result) => Exit::Dropped(result),
                    None => Exit::Cancelled(CancelReason::NoDropTarget),
                };
                self.end_drag(exit, cx);
            }
            Effect::Cancel(reason) => self.end_drag(Exit::Cancelled(reason), cx),
        }
    }

    /// The one place a drag ends. A drop, Escape, leaving the window, losing focus, and losing
    /// capture all come through here. It clears the timer, the focus watch, and the overlay, so
    /// nothing is left behind.
    fn end_drag(&mut self, exit: Exit, cx: &mut Context<Self>) {
        self.drag.timer = None;
        self.drag.activation = None;
        self.drag.overlay_width = None;
        self.drag.last_exit = Some(exit);
        cx.notify();
    }
}

/// Opens a task's detail view. There's no detail view yet, so a click does nothing visible.
fn open_card(_task: TaskId) {}
