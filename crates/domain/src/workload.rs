//! Workload: for a period, how much estimated work each person carries against the working time they have.
//!
//! The scoping rules, written down:
//! - A task counts if its due date falls inside the period (both ends included). A task that spans the
//!   edge of the period is counted by its due date. Nothing is prorated.
//! - Done tasks are left out unless the caller includes them.
//! - Tasks with no due date can't be placed in a period, so they're left out and counted.
//! - A task with no estimate counts as zero hours, and its count is reported.
//! - A task assigned to someone no longer on the team gets its own row. It's never dropped.
//! - Unassigned work gets its own row. It has no capacity, so it shows hours but no percentage.
//!
//! Hours are summed in whole minutes, and only the display rounds.

use crate::{
    ids::TeamMemberId,
    status::Status,
    task::Task,
    working_time::{Period, WorkingCalendar, available_minutes},
};

/// A load above this share of available hours is overloaded. Kept as a constant so it can become a
/// per-project setting later.
pub const OVERLOAD_PERCENT: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// A current team member.
    Member(TeamMemberId),
    /// Tasks assigned to someone who has since left the team.
    Removed(TeamMemberId),
    Unassigned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadRow {
    pub owner: Owner,
    pub estimated_minutes: i64,
    /// The working minutes this owner has in the period. `None` for unassigned and removed work, which
    /// has no one to measure against.
    pub available_minutes: Option<i64>,
    pub tasks: u32,
    pub unestimated_tasks: u32,
}

impl WorkloadRow {
    /// The load as a whole percent of available time, rounded to the nearest percent. `None` when there's
    /// nothing to measure against, or when the owner has no working time in the period.
    pub fn percent(&self) -> Option<i64> {
        let available = self.available_minutes.filter(|minutes| *minutes > 0)?;
        Some((self.estimated_minutes * 100 + available / 2) / available)
    }

    /// Whether the load is over the threshold. An owner with no working time who has estimated work is
    /// overloaded, since any work at all is more than their time.
    pub fn overloaded(&self) -> bool {
        match self.available_minutes {
            Some(0) => self.estimated_minutes > 0,
            Some(available) => self.estimated_minutes * 100 > available * OVERLOAD_PERCENT,
            None => false,
        }
    }

    /// Whether the owner has no working days in the period, so their load can't be a percentage.
    pub fn has_no_working_days(&self) -> bool {
        self.available_minutes == Some(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadReport {
    pub period: Period,
    /// Team members in the order given, then removed members, then unassigned work last.
    pub rows: Vec<WorkloadRow>,
    /// Candidate tasks due in the period.
    pub in_period: u32,
    /// Candidate tasks due outside the period.
    pub outside_period: u32,
    /// Candidate tasks with no due date, which can't be placed in any period.
    pub undated: u32,
}

impl WorkloadReport {
    /// The total estimated minutes across every row. Equal to the estimates of the tasks in the period.
    pub fn estimated_minutes(&self) -> i64 {
        self.rows.iter().map(|row| row.estimated_minutes).sum()
    }
}

/// The workload for a period.
///
/// `members` are the current team, in the order they should be shown. A task assigned to anyone else is
/// shown under `Owner::Removed`, with removed owners after the team in the order their work was found.
pub fn workload(
    tasks: &[Task],
    members: &[TeamMemberId],
    calendar: &WorkingCalendar,
    period: Period,
    include_done: bool,
) -> WorkloadReport {
    let available = available_minutes(calendar, period);
    let mut rows: Vec<WorkloadRow> = members
        .iter()
        .map(|id| WorkloadRow {
            owner: Owner::Member(*id),
            estimated_minutes: 0,
            available_minutes: Some(available),
            tasks: 0,
            unestimated_tasks: 0,
        })
        .collect();
    let mut unassigned = WorkloadRow {
        owner: Owner::Unassigned,
        estimated_minutes: 0,
        available_minutes: None,
        tasks: 0,
        unestimated_tasks: 0,
    };
    let (mut in_period, mut outside_period, mut undated) = (0, 0, 0);

    for task in tasks {
        if task.status() == Status::Done && !include_done {
            continue;
        }
        let Some(due) = task.due_on() else {
            undated += 1;
            continue;
        };
        if !period.contains(due) {
            outside_period += 1;
            continue;
        }
        in_period += 1;

        let owner = match task.assignee_id() {
            Some(id) if members.contains(&id) => Owner::Member(id),
            Some(id) => Owner::Removed(id),
            None => Owner::Unassigned,
        };
        let row = match owner {
            Owner::Member(_) => rows
                .iter_mut()
                .find(|row| row.owner == owner)
                .expect("every current member has a row"),
            Owner::Removed(_) => row_for_owner(&mut rows, owner),
            Owner::Unassigned => &mut unassigned,
        };
        row.estimated_minutes += task
            .estimate()
            .map_or(0, |estimate| i64::from(estimate.minutes()));
        row.tasks += 1;
        row.unestimated_tasks += u32::from(task.estimate().is_none());
    }

    // Current members always have a row. Removed owners and unassigned work appear only when they carry
    // something, so an empty row doesn't suggest there's work that isn't there.
    rows.retain(|row| match row.owner {
        Owner::Member(_) => true,
        Owner::Removed(_) | Owner::Unassigned => row.tasks > 0,
    });
    if unassigned.tasks > 0 {
        rows.push(unassigned);
    }

    WorkloadReport {
        period,
        rows,
        in_period,
        outside_period,
        undated,
    }
}

/// The row for a removed owner, made on first use and kept in the order their work was found.
fn row_for_owner(rows: &mut Vec<WorkloadRow>, owner: Owner) -> &mut WorkloadRow {
    let position = match rows.iter().position(|row| row.owner == owner) {
        Some(position) => position,
        None => {
            rows.push(WorkloadRow {
                owner,
                estimated_minutes: 0,
                available_minutes: None,
                tasks: 0,
                unestimated_tasks: 0,
            });
            rows.len() - 1
        }
    };
    &mut rows[position]
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        ids::{ProjectId, TeamMemberId},
        position::Position,
        status::Status,
        task::{Estimate, Task},
        task_fields::DueDate,
        working_time::{Period, WorkingCalendar},
        workload::{OVERLOAD_PERCENT, Owner, WorkloadReport, workload},
    };

    fn day(year: i32, month: u32, day: u32) -> DueDate {
        DueDate::new(year, month, day).unwrap()
    }

    /// A task due on `due`, with an estimate in hours and an optional assignee.
    fn task(due: Option<DueDate>, hours: Option<f64>, assignee: Option<TeamMemberId>) -> Task {
        let mut task = Task::new(
            ProjectId::new(),
            "Task",
            Status::Todo,
            Position::new("a0").unwrap(),
        )
        .unwrap();
        task.set_due_on(due);
        task.set_estimate(hours.map(|hours| Estimate::from_hours(hours).unwrap()));
        task.assign(assignee);
        task
    }

    fn row_for(report: &WorkloadReport, owner: Owner) -> &super::WorkloadRow {
        report
            .rows
            .iter()
            .find(|row| row.owner == owner)
            .expect("the owner has a row")
    }

    /// The hand-calculated sample: Monday 12 October to Friday 16 October 2026.
    #[test]
    fn the_hand_calculated_week_matches_the_totals() {
        let alice = TeamMemberId::new();
        let ben = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let due = Some(day(2026, 10, 14));
        let tasks = [
            // Alice: 8 + 16 + 20 = 44 hours.
            task(due, Some(8.0), Some(alice)),
            task(due, Some(16.0), Some(alice)),
            task(due, Some(20.0), Some(alice)),
            // Ben: 24 hours.
            task(due, Some(24.0), Some(ben)),
            // Nobody: 12 hours.
            task(due, Some(12.0), None),
        ];

        let report = workload(
            &tasks,
            &[alice, ben],
            &WorkingCalendar::stub(),
            period,
            false,
        );

        // Five working days at eight hours is 40 hours for each person.
        let alice_row = row_for(&report, Owner::Member(alice));
        assert_eq!(alice_row.available_minutes, Some(40 * 60));
        assert_eq!(alice_row.estimated_minutes, 44 * 60);
        assert_eq!(alice_row.percent(), Some(110));
        assert!(alice_row.overloaded());

        let ben_row = row_for(&report, Owner::Member(ben));
        assert_eq!(ben_row.estimated_minutes, 24 * 60);
        assert_eq!(ben_row.percent(), Some(60));
        assert!(!ben_row.overloaded());

        let unassigned = row_for(&report, Owner::Unassigned);
        assert_eq!(unassigned.estimated_minutes, 12 * 60);
        assert_eq!(
            unassigned.available_minutes, None,
            "no one to measure it against"
        );
        assert_eq!(unassigned.percent(), None);
    }

    #[test]
    fn the_rows_add_up_to_every_in_scope_estimate() {
        let alice = TeamMemberId::new();
        let removed = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let due = Some(day(2026, 10, 13));
        let tasks = [
            task(due, Some(8.0), Some(alice)),
            task(due, Some(3.5), Some(removed)),
            task(due, Some(2.25), None),
            task(due, None, Some(alice)),
            // Outside the period, and undated, so none of these count.
            task(Some(day(2026, 11, 2)), Some(100.0), Some(alice)),
            task(None, Some(100.0), None),
        ];
        let in_scope: i64 = tasks
            .iter()
            .filter(|task| task.due_on().is_some_and(|due| period.contains(due)))
            .map(|task| task.estimate().map_or(0, |e| i64::from(e.minutes())))
            .sum();

        let report = workload(&tasks, &[alice], &WorkingCalendar::stub(), period, false);

        assert_eq!(report.estimated_minutes(), in_scope);
        assert_eq!(in_scope, (8 * 60) + 210 + 135, "8h, 3.5h, and 2.25h");
    }

    #[test]
    fn a_weekend_period_has_no_working_days_and_no_percentage() {
        let alice = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 17), day(2026, 10, 18)).unwrap();
        let tasks = [task(Some(day(2026, 10, 17)), Some(4.0), Some(alice))];

        let report = workload(&tasks, &[alice], &WorkingCalendar::stub(), period, false);
        let row = row_for(&report, Owner::Member(alice));

        assert!(row.has_no_working_days());
        assert_eq!(row.percent(), None, "no percentage without working time");
        assert!(
            row.overloaded(),
            "work with no time at all is over capacity"
        );
    }

    #[test]
    fn a_member_with_no_work_still_has_a_row() {
        let alice = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();

        let report = workload(&[], &[alice], &WorkingCalendar::stub(), period, false);

        let row = row_for(&report, Owner::Member(alice));
        assert_eq!(row.estimated_minutes, 0);
        assert_eq!(row.percent(), Some(0));
    }

    #[test]
    fn done_tasks_are_left_out_unless_asked_for() {
        let alice = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let mut done = task(Some(day(2026, 10, 13)), Some(5.0), Some(alice));
        done.set_status(Status::Done);
        let tasks = [done];

        let without = workload(&tasks, &[alice], &WorkingCalendar::stub(), period, false);
        let with = workload(&tasks, &[alice], &WorkingCalendar::stub(), period, true);

        assert_eq!(row_for(&without, Owner::Member(alice)).estimated_minutes, 0);
        assert_eq!(
            row_for(&with, Owner::Member(alice)).estimated_minutes,
            5 * 60
        );
    }

    #[test]
    fn undated_and_unestimated_tasks_are_counted_not_hidden() {
        let alice = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let tasks = [
            task(Some(day(2026, 10, 13)), None, Some(alice)),
            task(None, Some(2.0), Some(alice)),
            task(Some(day(2026, 12, 1)), Some(2.0), Some(alice)),
        ];

        let report = workload(&tasks, &[alice], &WorkingCalendar::stub(), period, false);

        assert_eq!(report.undated, 1);
        assert_eq!(report.outside_period, 1);
        assert_eq!(report.in_period, 1);
        let row = row_for(&report, Owner::Member(alice));
        assert_eq!(
            row.unestimated_tasks, 1,
            "it counts as zero hours and is reported"
        );
        assert_eq!(row.estimated_minutes, 0);
    }

    #[test]
    fn a_removed_members_work_gets_its_own_row_and_is_not_dropped() {
        let gone = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let tasks = [task(Some(day(2026, 10, 13)), Some(6.0), Some(gone))];

        let report = workload(&tasks, &[], &WorkingCalendar::stub(), period, false);

        let row = row_for(&report, Owner::Removed(gone));
        assert_eq!(row.estimated_minutes, 6 * 60);
        assert_eq!(row.percent(), None);
    }

    #[test]
    fn the_overload_threshold_is_a_named_constant_at_one_hundred_percent() {
        assert_eq!(OVERLOAD_PERCENT, 100);
        let alice = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let exactly_full = [task(Some(day(2026, 10, 13)), Some(40.0), Some(alice))];

        let report = workload(
            &exactly_full,
            &[alice],
            &WorkingCalendar::stub(),
            period,
            false,
        );

        let row = row_for(&report, Owner::Member(alice));
        assert_eq!(row.percent(), Some(100));
        assert!(!row.overloaded(), "exactly full is not over");
    }

    #[test]
    fn a_task_for_a_member_listed_twice_is_counted_once() {
        let alice = TeamMemberId::new();
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let tasks = [task(Some(day(2026, 10, 13)), Some(3.0), Some(alice))];

        let report = workload(
            &tasks,
            &[alice, alice],
            &WorkingCalendar::stub(),
            period,
            false,
        );

        assert_eq!(
            report.estimated_minutes(),
            3 * 60,
            "the sum never double counts"
        );
    }
}
