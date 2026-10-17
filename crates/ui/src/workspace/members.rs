//! How team members appear in the UI: the label each one is shown under, its initials and colour,
//! and the wording for a removal. The labels come from the current member list every time, so they
//! stay right after a rename or a removal.

use looji_domain::{MemberColour, TeamMember, TeamMemberId, display_labels};

/// One member as the UI shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: TeamMemberId,
    /// The name, with a place suffix such as "(2)" when another member has the same name.
    pub label: String,
    /// The full name, without a suffix.
    pub name: String,
    pub initials: String,
    pub colour: MemberColour,
    /// A short form of the stable ID, shown with the name in the tooltip.
    pub short_id: String,
}

/// The members in the order given, which is the order they were added in.
pub fn entries(members: &[TeamMember]) -> Vec<Entry> {
    display_labels(members)
        .into_iter()
        .zip(members)
        .map(|(label, member)| Entry {
            id: member.id(),
            label,
            name: member.name().to_string(),
            initials: member.initials(),
            colour: member.colour(),
            short_id: member.id().as_uuid().simple().to_string()[..8].to_string(),
        })
        .collect()
}

/// What the removal confirmation says about the tasks the member has.
pub fn unassigned_message(tasks: u64) -> String {
    match tasks {
        0 => "No tasks are assigned to them.".to_string(),
        1 => "1 task will become unassigned.".to_string(),
        count => format!("{count} tasks will become unassigned."),
    }
}

/// The tooltip for a member: the full name, and the ID that tells two same-named members apart.
pub fn tooltip(entry: &Entry) -> String {
    format!("{} ({})", entry.name, entry.short_id)
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{MemberColour, TeamMember, WorkspaceId};

    use super::{entries, tooltip, unassigned_message};

    fn member(name: &str, existing: usize) -> TeamMember {
        TeamMember::new(
            WorkspaceId::new(),
            name,
            MemberColour::for_new_member(existing),
        )
        .unwrap()
    }

    #[test]
    fn two_members_with_the_same_name_are_told_apart_by_label_and_colour() {
        let members = [member("Alex Kim", 0), member("Alex Kim", 1)];
        let shown = entries(&members);

        assert_eq!(shown[0].label, "Alex Kim (1)");
        assert_eq!(shown[1].label, "Alex Kim (2)");
        assert_eq!(shown[0].initials, shown[1].initials, "the initials match");
        assert_ne!(
            shown[0].colour, shown[1].colour,
            "so the colour has to tell them apart"
        );
        assert_ne!(shown[0].id, shown[1].id);
    }

    #[test]
    fn a_tooltip_gives_the_full_name_and_the_id() {
        let members = [member("Alex Kim", 0), member("Alex Kim", 1)];
        let shown = entries(&members);

        assert_eq!(
            tooltip(&shown[0]),
            format!("Alex Kim ({})", shown[0].short_id)
        );
        assert_ne!(tooltip(&shown[0]), tooltip(&shown[1]));
    }

    #[test]
    fn removing_one_of_two_same_named_members_drops_the_suffix() {
        let members = [member("Alex Kim", 0), member("Alex Kim", 1)];
        let remaining = entries(&members[1..]);
        assert_eq!(remaining[0].label, "Alex Kim");
    }

    #[test]
    fn a_rename_changes_the_label_without_touching_the_colour() {
        let mut members = [member("Alex Kim", 0), member("Alex Kim", 1)];
        let before = entries(&members);

        members[1].rename("Alex Park").unwrap();
        let after = entries(&members);

        assert_eq!(
            after[0].label, "Alex Kim",
            "the suffix goes once the name is unique"
        );
        assert_eq!(after[1].label, "Alex Park");
        assert_eq!(after[1].colour, before[1].colour);
    }

    #[test]
    fn the_removal_message_counts_the_tasks() {
        assert_eq!(unassigned_message(0), "No tasks are assigned to them.");
        assert_eq!(unassigned_message(1), "1 task will become unassigned.");
        assert_eq!(unassigned_message(3), "3 tasks will become unassigned.");
    }
}
