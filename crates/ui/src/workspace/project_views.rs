//! Each project's view state: the view it was showing, and where each scrollable place was scrolled.
//!
//! The filters, search, and list paging already live on the workspace view, not inside any one view, so
//! switching views keeps them. This file owns the other part: which view a project opens on, and the
//! scroll positions to return to.
//!
//! Scroll positions are read from the live handles when the user leaves a view, and when the app quits. They
//! are written to the database at those moments, not on every scroll event, so scrolling costs nothing on
//! the database. The cost of that choice: if the app crashes, the scroll position of the view it was on is
//! lost, though the view itself is kept.
//!
//! Restoring a position is done after the view has laid out, on the next frame, and clamped to the content's
//! height, since the data may have changed while the user was away.

use gpui::{prelude::*, *};
use looji_domain::{ProjectViewRepository, ProjectViewState, ScrollOffset, scroll_key};

use super::list_view::ViewMode;
use super::{WorkspaceView, store};

/// A scroll handle's position as the distances the content has been moved, which are zero or more.
fn offset_of(handle: &ScrollHandle) -> ScrollOffset {
    let offset = handle.offset();
    ScrollOffset {
        x: (-f32::from(offset.x)).max(0.),
        y: (-f32::from(offset.y)).max(0.),
    }
}

impl WorkspaceView {
    /// The scrollable places a view has, and the handle for each.
    fn places(&self, view: ViewMode) -> Vec<(String, ScrollHandle)> {
        match view {
            ViewMode::Board => self
                .projects
                .active()
                .map(|project| project.visible_statuses())
                .unwrap_or_default()
                .into_iter()
                .map(|status| {
                    (
                        scroll_key::board_column(status.as_str()),
                        self.drag.scroll_handle(status),
                    )
                })
                .collect(),
            ViewMode::List => vec![(scroll_key::LIST.to_string(), self.scroll_list.clone())],
            ViewMode::Roadmap => {
                vec![(scroll_key::ROADMAP.to_string(), self.scroll_roadmap.clone())]
            }
            ViewMode::Workload => vec![(
                scroll_key::WORKLOAD.to_string(),
                self.scroll_workload.clone(),
            )],
        }
    }

    /// Records where a view is scrolled now, into the active project's state.
    pub(super) fn capture_scroll(&mut self, view: ViewMode) {
        for (key, handle) in self.places(view) {
            self.view_scroll.insert(key, offset_of(&handle));
        }
    }

    /// Writes the active project's view and scroll positions, if they've changed since they were loaded.
    /// A project that's only been looked at writes nothing.
    pub(super) fn save_project_view(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        self.capture_scroll(self.view_mode);
        let state = ProjectViewState {
            view: self.view_mode,
            scroll: self.view_scroll.clone(),
        };
        if state == self.loaded_view_state {
            return;
        }
        match store(cx).save_view_state(project, &state) {
            Ok(()) => self.loaded_view_state = state,
            Err(err) => {
                eprintln!(
                    "could not save the view for project {}: {err}",
                    project.as_uuid()
                );
                self.status = Some("Could not save this board's view.".to_string());
            }
        }
    }

    /// Loads the active project's view and scroll positions. A project with none opens on the board, at the
    /// top. The positions are applied after the view lays out.
    pub(super) fn load_project_view(&mut self, cx: &mut Context<Self>) {
        let state = match self.projects.active_id() {
            Some(project) => store(cx).load_view_state(project).unwrap_or_else(|err| {
                eprintln!(
                    "could not load the view for project {}: {err}",
                    project.as_uuid()
                );
                ProjectViewState::default()
            }),
            None => ProjectViewState::default(),
        };
        self.view_mode = state.view;
        self.view_scroll = state.scroll.clone();
        self.loaded_view_state = state;
        self.pending_scroll = true;
    }

    /// Puts the scroll positions back once the view has laid out. Each place gets its saved position, or the
    /// top if it has none, so a previous project's position doesn't carry over.
    pub(super) fn schedule_scroll_restore(&mut self, window: &mut Window) {
        let targets: Vec<(ScrollHandle, ScrollOffset)> = self
            .places(self.view_mode)
            .into_iter()
            .map(|(key, handle)| {
                let saved = self.view_scroll.get(&key).copied().unwrap_or_default();
                (handle, saved)
            })
            .collect();
        window.on_next_frame(move |_, _| {
            for (handle, saved) in targets {
                // The maximum is only known once the content has been laid out, which is why this waits for
                // the next frame. Clamping keeps a position from pointing past content that has shrunk.
                let max = handle.max_offset();
                let x = saved.x.min(f32::from(max.x)).max(0.);
                let y = saved.y.min(f32::from(max.y)).max(0.);
                handle.set_offset(point(px(-x), px(-y)));
            }
        });
    }

    /// Records the active project's state when the app quits, so the next launch opens where the user left.
    pub(super) fn save_on_quit(&mut self, cx: &mut Context<Self>) {
        self.save_project_view(cx);
    }
}
