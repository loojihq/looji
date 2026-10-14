//! The pointer side of drag. It feeds window input to the drag session in `drag.rs`, starts the
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
use super::autoscroll::{EDGE_ZONE, Ramp, advance, edge_velocity};
use super::board::{card_context, card_view};
use super::card::{CalendarDate, CardDensity, task_card};
use super::drag::{CancelReason, Command, Drag, Exit, Input};
use super::keyboard::Hold;
use super::layout::{BoardLayout, CardSlot, ColumnSlot, Point as DragPoint, Rect};
use crate::theme::Theme;

/// How often the scroll loop runs while a column is being scrolled.
const SCROLL_TICK: Duration = Duration::from_millis(16);

/// The drag state owned by the workspace view.
pub(super) struct DragSession {
    pub(super) session: Drag,
    /// The long-press timer. Dropping it cancels the timer.
    timer: Option<Task<()>>,
    /// Ends the drag if the window stops being active. Dropped when the drag ends.
    activation: Option<Subscription>,
    /// Width of the lifted card, taken from the slot it came from.
    overlay_width: Option<Pixels>,
    /// Filled by the board's paint. Shared with the paint callbacks.
    pub(super) layout: Rc<RefCell<BoardLayout>>,
    /// One scroll handle per status column, kept across renders so each column keeps its place.
    scrolls: Vec<(Status, ScrollHandle)>,
    /// Where the pointer is, for the scroll loop to re-check the landing against.
    pointer: Option<DragPoint>,
    /// The column the pointer is over, which is the only one that scrolls.
    scroll_owner: Option<Status>,
    /// How far the scroll speed has ramped in or out.
    ramp: Ramp,
    /// The scroll loop. Dropping it cancels the loop.
    scroll_task: Option<Task<()>>,
    /// Whether a scroll loop is running, so only one runs at a time.
    scrolling: bool,
    #[cfg_attr(not(debug_assertions), allow(dead_code))] // Read by the debug controls only.
    last_exit: Option<Exit>,
    /// The keyboard move in progress, if any.
    pub(super) keyboard: Hold,
    /// A card the press opened, for the release to hand to the edit dialog.
    pub(super) opened: Option<TaskId>,
    /// What the last keyboard step said. Shown on the board, and the text a screen reader would read.
    pub(super) keyboard_notice: Option<String>,
    /// Cancels a keyboard move if the window stops being active.
    pub(super) keyboard_activation: Option<Subscription>,
    /// One focus handle per card, kept by task id, so focus follows a card when it moves.
    card_focus: RefCell<Vec<(TaskId, FocusHandle)>>,
}

impl Default for DragSession {
    fn default() -> Self {
        Self {
            session: Drag::default(),
            timer: None,
            activation: None,
            overlay_width: None,
            layout: Rc::default(),
            scrolls: Status::ALL
                .into_iter()
                .map(|status| (status, ScrollHandle::new()))
                .collect(),
            pointer: None,
            scroll_owner: None,
            ramp: Ramp::default(),
            scroll_task: None,
            scrolling: false,
            last_exit: None,
            keyboard: Hold::Idle,
            opened: None,
            keyboard_notice: None,
            keyboard_activation: None,
            card_focus: RefCell::default(),
        }
    }
}

impl DragSession {
    /// The drag as it stands: the source, and the landing the placeholder shows.
    pub(super) fn session(&self) -> &Drag {
        &self.session
    }

    /// The scroll handle of a status column. The column's body tracks it.
    pub(super) fn scroll_handle(&self, status: Status) -> ScrollHandle {
        self.scrolls
            .iter()
            .find(|(column, _)| *column == status)
            .map(|(_, handle)| handle.clone())
            .expect("every status has a scroll handle")
    }

    /// The card being dragged, once it has lifted. The board dims its slot.
    /// The card being dragged or held. Set only while it is lifted or held, so the board dims it
    /// then, and nothing else.
    pub(super) fn dragged(&self) -> Option<TaskId> {
        self.session.source().map(|source| source.task)
    }

    /// The focus handle of a card. Created the first time the card is drawn, then kept.
    pub(super) fn focus_handle(&self, task: TaskId, cx: &App) -> FocusHandle {
        let mut handles = self.card_focus.borrow_mut();
        if let Some((_, handle)) = handles.iter().find(|(id, _)| *id == task) {
            return handle.clone();
        }
        let handle = cx.focus_handle().tab_stop(true).tab_index(0);
        handles.push((task, handle.clone()));
        handle
    }

    pub(super) fn keyboard_notice(&self) -> Option<&str> {
        self.keyboard_notice.as_deref()
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

fn point(position: Point<Pixels>) -> DragPoint {
    DragPoint {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

/// A painted rectangle, in the drag module's terms.
fn rect(bounds: Bounds<Pixels>) -> Rect {
    Rect {
        origin: DragPoint {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
        },
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

/// The painted columns, in the order the board lays them out.
pub(super) fn record_columns(
    layout: &Rc<RefCell<BoardLayout>>,
    statuses: Vec<Status>,
) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static {
    let layout = layout.clone();
    move |bounds, _, _| {
        let columns = statuses
            .iter()
            .zip(bounds)
            .map(|(&status, bounds)| ColumnSlot {
                status,
                bounds: rect(bounds),
            })
            .collect();
        layout.borrow_mut().set_columns(columns);
    }
}

/// Works out where every card is, from each column's scroll state, and records them. The
/// paint doesn't report card bounds while a column is scroll-tracked, so the positions come from
/// the scroll handle: where the item was laid out, moved by the column's current offset.
fn refresh_cards(layout: &mut BoardLayout, handle: impl Fn(Status) -> ScrollHandle) {
    let updates: Vec<(Status, Vec<CardSlot>)> = layout
        .items
        .iter()
        .map(|(status, items)| {
            let handle = handle(*status);
            let offset = handle.offset();
            let cards = items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let task = (*item)?;
                    let bounds = handle.bounds_for_item(index)?;
                    Some(CardSlot {
                        task,
                        status: *status,
                        bounds: rect(Bounds {
                            origin: bounds.origin + offset,
                            size: bounds.size,
                        }),
                    })
                })
                .collect();
            (*status, cards)
        })
        .collect();
    for (status, cards) in updates {
        layout.set_cards(status, cards);
    }
}

/// Whether a point is inside the window. Mouse moves and releases still arrive outside it
/// while the button is held, so this is how leaving the window is detected.
fn inside(window: &Window, point: DragPoint) -> bool {
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
    let (card, top_left) = view.drag.session.lifted_at()?;
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
                card_view(task, &card_context(view)),
                CardDensity::Compact,
                today,
                theme,
                None,
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
        self.refresh_layout();
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
        if self.drag.activation.is_none() {
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
        self.follow_scroll(pointer, cx);
    }

    /// Records where the pointer is and which column owns the scroll. Moving to another column
    /// stops scrolling the old one at once, and the new column's speed ramps in from zero.
    fn follow_scroll(&mut self, pointer: DragPoint, cx: &mut Context<Self>) {
        self.drag.pointer = Some(pointer);
        let owner = self
            .drag
            .layout
            .borrow()
            .column_at(pointer)
            .map(|column| column.status);
        if owner != self.drag.scroll_owner {
            self.drag.scroll_owner = owner;
            self.drag.ramp.reset();
        }
        if owner.is_some() && !self.drag.scrolling {
            self.start_autoscroll(cx);
        }
    }

    /// Starts the scroll loop. It runs until the pointer is out of every zone and the speed has
    /// ramped back down, or until the drag ends.
    fn start_autoscroll(&mut self, cx: &mut Context<Self>) {
        self.drag.scrolling = true;
        self.drag.scroll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SCROLL_TICK).await;
                let keep_going = this
                    .update(cx, |view, cx| view.autoscroll_tick(cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    /// One step of the scroll loop. Returns whether the loop should keep running.
    fn autoscroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pointer) = self.drag.pointer else {
            self.drag.scrolling = false;
            return false;
        };
        if self.drag.session.lifted_at().is_none() {
            self.drag.scrolling = false;
            return false;
        }
        let owner = self.drag.scroll_owner;
        let velocity = owner.map_or(0.0, |status| self.edge_velocity_for(status, pointer));
        let inside = velocity != 0.0;
        let factor = self.drag.ramp.step(inside, SCROLL_TICK.as_secs_f32());
        if !inside && factor == 0.0 {
            self.drag.scrolling = false;
            return false;
        }
        if let Some(status) = owner {
            let seconds = SCROLL_TICK.as_secs_f32();
            self.scroll_column(status, velocity * factor * seconds, cx);
        }
        true
    }

    /// The scroll speed for `pointer` over a column's body. Zero if the column doesn't overflow,
    /// so it has nothing to scroll.
    fn edge_velocity_for(&self, status: Status, pointer: DragPoint) -> f32 {
        let handle = self.drag.scroll_handle(status);
        if f32::from(handle.max_offset().height) <= 0.0 {
            return 0.0;
        }
        let body = handle.bounds();
        edge_velocity(
            pointer.y,
            f32::from(body.origin.y),
            f32::from(body.origin.y + body.size.height),
            EDGE_ZONE,
        )
    }

    /// Scrolls a column by `delta` pixels, clamped at both ends. The painted cards move with it at
    /// once, and the landing is worked out again from the moved positions.
    fn scroll_column(&mut self, status: Status, delta: f32, cx: &mut Context<Self>) {
        let handle = self.drag.scroll_handle(status);
        let max = f32::from(handle.max_offset().height);
        let scroll = -f32::from(handle.offset().y);
        let next = advance(scroll, max, delta);
        let applied = next - scroll;
        if applied == 0.0 {
            return;
        }
        let offset = handle.offset();
        handle.set_offset(Point {
            x: offset.x,
            y: px(-next),
        });
        if let Some(pointer) = self.drag.pointer {
            self.drive_drag(Input::Move { pointer, at: now() }, cx);
        }
        cx.notify();
    }

    fn pointer_released(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drag.activation.is_none() || event.button != MouseButton::Left {
            return;
        }
        let pointer = point(event.position);
        let input = if inside(window, pointer) {
            Input::Up { pointer }
        } else {
            Input::Cancel(CancelReason::ReleasedOutside)
        };
        self.drive_drag(input, cx);
        // A click on a card, with no drag, opens it for editing.
        if let Some(task) = self.drag.opened.take() {
            self.open_panel(task, cx);
        }
    }

    /// Escape cancels a drag. Returns whether the key was used. Keys go through the focused
    /// element, so this is called from the workspace's key handler rather than a window listener.
    pub(super) fn key_pressed(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.drag.activation.is_none() || event.keystroke.key != "escape" {
            return false;
        }
        self.drive_drag(Input::Cancel(CancelReason::Escape), cx);
        true
    }

    /// Re-reads the cards from their columns' scroll state, so the drag sees what is on screen.
    pub(super) fn refresh_layout(&self) {
        let mut layout = self.drag.layout.borrow_mut();
        refresh_cards(&mut layout, |status| self.drag.scroll_handle(status));
    }

    /// Feeds one input to the drag and carries out what it asks for.
    fn drive_drag(&mut self, input: Input, cx: &mut Context<Self>) {
        self.refresh_layout();
        let layout = self.drag.layout.clone();
        let commands = self.drag.session.feed(input, &layout.borrow());
        for command in commands {
            self.apply_command(command, cx);
        }
    }

    fn apply_command(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::ArmTimer { generation, after } => {
                self.drag.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(after).await;
                    let _ = this.update(cx, |view, cx| {
                        view.drive_drag(Input::Tick { generation }, cx)
                    });
                }));
            }
            Command::OpenCard(task) => {
                // A click without a drag ends the press here. Clearing the drag's watch lets keys
                // reach the rest of the app, such as Escape closing a dialog.
                self.drag.opened = Some(task);
                self.drag.activation = None;
                self.drag.timer = None;
            }
            Command::Lift(card) => {
                self.drag.overlay_width = self
                    .drag
                    .layout
                    .borrow()
                    .card(card)
                    .map(|slot| px(slot.bounds.width));
                cx.notify();
            }
            Command::Follow => cx.notify(),
            Command::End(exit) => self.end_drag(exit, cx),
        }
    }

    /// The one place a drag ends. A drop, Escape, leaving the window, losing focus, and losing
    /// capture all come through here. It clears the timer, the focus watch, and the overlay, so
    /// nothing is left behind.
    fn end_drag(&mut self, exit: Exit, cx: &mut Context<Self>) {
        self.drag.timer = None;
        self.drag.activation = None;
        self.drag.overlay_width = None;
        self.drag.scroll_task = None;
        self.drag.scrolling = false;
        self.drag.scroll_owner = None;
        self.drag.pointer = None;
        self.drag.ramp.reset();
        self.drag.last_exit = Some(exit);
        // Placed at a new index, in this column or another. A drop where the card already is
        // never reaches here: it ends as `Exit::Unchanged`.
        if let Exit::Dropped(result) = exit {
            self.commit_move(result, cx);
        }
        cx.notify();
    }
}
