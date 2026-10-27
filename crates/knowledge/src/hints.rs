//! Hints for a task from similar tasks the team has already done: about how long it will take, and who has worked on
//! work like it. These are offered, never applied. The rules are plain and strict: a hint needs enough similar tasks, or
//! one very close one, so a weak match says nothing rather than something misleading.

use std::collections::BTreeMap;

/// A similar task, as far as a hint needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastTask {
    /// How alike it is to the task being hinted, 0 to 100.
    pub percent: u32,
    pub done: bool,
    pub minutes: Option<u32>,
    /// Who has it, as any stable key, such as the member's id.
    pub assignee: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hints {
    /// The usual time for work like this, rounded to a quarter of an hour.
    pub estimate_minutes: Option<u32>,
    /// How many finished tasks it comes from.
    pub estimate_from: usize,
    /// Who has worked on work like this most.
    pub assignee: Option<String>,
    /// How many similar tasks that person has.
    pub assignee_from: usize,
}

/// A task has to be at least this alike to count at all.
const MIN_PERCENT: u32 = 30;
/// A single task this alike is enough on its own.
const CLOSE_PERCENT: u32 = 60;
/// Estimates are rounded to this many minutes.
const ROUND_TO: u32 = 15;

fn round(minutes: f64) -> u32 {
    let rounded = (minutes / f64::from(ROUND_TO)).round() as u32 * ROUND_TO;
    rounded.max(ROUND_TO)
}

/// The hints the similar tasks support.
pub fn hints(similar: &[PastTask]) -> Hints {
    let counted: Vec<&PastTask> = similar
        .iter()
        .filter(|task| task.percent >= MIN_PERCENT)
        .collect();
    let mut result = Hints::default();

    let mut times: Vec<u32> = counted
        .iter()
        .filter(|task| task.done)
        .filter_map(|task| task.minutes.filter(|minutes| *minutes > 0))
        .collect();
    let close = counted
        .iter()
        .any(|task| task.done && task.minutes.is_some() && task.percent >= CLOSE_PERCENT);
    if times.len() >= 2 || (times.len() == 1 && close) {
        times.sort_unstable();
        let middle = times.len() / 2;
        let median = if times.len() % 2 == 1 {
            f64::from(times[middle])
        } else {
            f64::from(times[middle - 1] + times[middle]) / 2.0
        };
        result.estimate_minutes = Some(round(median));
        result.estimate_from = times.len();
    }

    // Who has the most similar work, weighted by how similar it is. Done or not, they know the area.
    let mut by_person: BTreeMap<&str, (u32, usize)> = BTreeMap::new();
    for task in &counted {
        if let Some(person) = task.assignee.as_deref() {
            let entry = by_person.entry(person).or_default();
            entry.0 += task.percent;
            entry.1 += 1;
        }
    }
    let best = by_person
        .into_iter()
        .max_by_key(|(person, (weight, count))| (*weight, *count, std::cmp::Reverse(*person)));
    if let Some((person, (weight, count))) = best
        && (count >= 2 || weight >= CLOSE_PERCENT)
    {
        result.assignee = Some(person.to_string());
        result.assignee_from = count;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(percent: u32, minutes: u32, who: &str) -> PastTask {
        PastTask {
            percent,
            done: true,
            minutes: Some(minutes),
            assignee: Some(who.to_string()),
        }
    }

    #[test]
    fn two_similar_finished_tasks_give_their_middle_time_rounded_to_a_quarter_hour() {
        let found = hints(&[done(45, 120, "a"), done(40, 200, "b")]);
        assert_eq!(found.estimate_minutes, Some(165));
        assert_eq!(found.estimate_from, 2);
    }

    #[test]
    fn the_median_ignores_one_wild_outlier() {
        let found = hints(&[done(50, 120, "a"), done(50, 130, "a"), done(50, 2400, "a")]);
        assert_eq!(found.estimate_minutes, Some(135));
    }

    #[test]
    fn one_weak_match_gives_no_estimate_but_one_close_match_does() {
        assert_eq!(hints(&[done(45, 120, "a")]).estimate_minutes, None);
        assert_eq!(hints(&[done(70, 120, "a")]).estimate_minutes, Some(120));
    }

    #[test]
    fn tasks_that_are_not_finished_or_barely_alike_give_no_time() {
        let open = PastTask {
            percent: 80,
            done: false,
            minutes: Some(60),
            assignee: None,
        };
        assert_eq!(hints(&[open.clone(), open]).estimate_minutes, None);
        assert_eq!(
            hints(&[done(10, 60, "a"), done(20, 90, "a")]),
            Hints::default()
        );
    }

    #[test]
    fn the_person_with_the_most_similar_work_is_suggested() {
        let open = |who: &str, percent| PastTask {
            percent,
            done: false,
            minutes: None,
            assignee: Some(who.into()),
        };
        let found = hints(&[open("sam", 40), open("sam", 35), open("maya", 50)]);
        assert_eq!(found.assignee.as_deref(), Some("sam"));
        assert_eq!(found.assignee_from, 2);
    }

    #[test]
    fn one_weak_link_to_a_person_is_not_enough_but_a_close_one_is() {
        let one = |percent| PastTask {
            percent,
            done: true,
            minutes: None,
            assignee: Some("sam".into()),
        };
        assert_eq!(hints(&[one(40)]).assignee, None);
        assert_eq!(hints(&[one(65)]).assignee.as_deref(), Some("sam"));
    }

    #[test]
    fn nothing_similar_gives_no_hints() {
        assert_eq!(hints(&[]), Hints::default());
    }
}
