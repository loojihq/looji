//! Team members: the people a project's tasks can be assigned to. Members are local names within
//! one project. They are not user accounts, so there are no logins or invitations.

use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    error::DomainError,
    ids::{ProjectId, TeamMemberId},
    time::Timestamp,
};

/// Longest member name, counted in characters after trimming and joining the spaces between words.
pub const MAX_MEMBER_NAME_CHARS: usize = 60;

/// How many colours a member can have. The UI maps each index to a palette entry.
pub const MEMBER_COLOUR_COUNT: usize = 6;

/// A member's colour, as an index into the palette. It's chosen once, when the member is added,
/// and stored, so renaming a member never changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemberColour(u8);

impl MemberColour {
    /// The colour for a member added when `existing` members are already in the project. Colours
    /// run in order, so the first members get distinct colours until the palette runs out.
    pub fn for_new_member(existing: usize) -> Self {
        Self((existing % MEMBER_COLOUR_COUNT) as u8)
    }

    /// Reads a stored index. An index outside the palette is refused.
    pub fn from_stored(index: i64) -> Result<Self, DomainError> {
        match u8::try_from(index) {
            Ok(index) if usize::from(index) < MEMBER_COLOUR_COUNT => Ok(Self(index)),
            _ => Err(DomainError::UnknownColour(index)),
        }
    }

    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// Trims the name, joins its inner runs of spaces into one, and checks the length. Names may repeat,
/// so there's no uniqueness check here.
pub fn validate_member_name(name: &str) -> Result<String, DomainError> {
    let cleaned = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return Err(DomainError::EmptyName);
    }
    if cleaned.chars().count() > MAX_MEMBER_NAME_CHARS {
        return Err(DomainError::TooLong {
            max: MAX_MEMBER_NAME_CHARS,
        });
    }
    Ok(cleaned)
}

/// Initials for a name, shown in the member's chip. The first letter of each of the first two
/// words, uppercased. A one-word name gives its first two letters. Letters are whole graphemes, so
/// an accent or an emoji stays with its letter.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let letters: Vec<&str> = match words.as_slice() {
        [] => return String::new(),
        [only] => only.graphemes(true).take(2).collect(),
        [first, second, ..] => first
            .graphemes(true)
            .take(1)
            .chain(second.graphemes(true).take(1))
            .collect(),
    };
    letters.concat().to_uppercase()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMember {
    id: TeamMemberId,
    project_id: ProjectId,
    name: String,
    colour: MemberColour,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl TeamMember {
    /// Adds a member. `colour` is fixed for the member's life.
    pub fn new(
        project_id: ProjectId,
        name: &str,
        colour: MemberColour,
    ) -> Result<Self, DomainError> {
        let now = Timestamp::now();
        Ok(Self {
            id: TeamMemberId::new(),
            project_id,
            name: validate_member_name(name)?,
            colour,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn id(&self) -> TeamMemberId {
        self.id
    }

    pub fn project_id(&self) -> ProjectId {
        self.project_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn colour(&self) -> MemberColour {
        self.colour
    }

    pub fn initials(&self) -> String {
        initials(&self.name)
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// Leaves the member unchanged if `name` is invalid.
    pub fn rename(&mut self, name: &str) -> Result<(), DomainError> {
        self.name = validate_member_name(name)?;
        self.updated_at = Timestamp::now();
        Ok(())
    }
}

/// Labels for a list of members, in the same order. A name shared by two or more members (ignoring
/// case) gets each member's place among them as a suffix, such as "Alex Kim (1)" and "Alex Kim
/// (2)". Other names are unchanged. Computed from the list each time, so renames and removals keep
/// the suffixes right. Pass the members in the order they were added, so the suffixes are stable.
pub fn display_labels(members: &[TeamMember]) -> Vec<String> {
    let mut totals: HashMap<String, usize> = HashMap::new();
    for member in members {
        *totals.entry(member.name().to_lowercase()).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    members
        .iter()
        .map(|member| {
            let key = member.name().to_lowercase();
            let place = seen.entry(key.clone()).or_default();
            *place += 1;
            if totals[&key] > 1 {
                format!("{} ({place})", member.name())
            } else {
                member.name().to_string()
            }
        })
        .collect()
}

/// Stored values for a team member. Converting it back into a [`TeamMember`] re-runs validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMemberRecord {
    pub id: TeamMemberId,
    pub project_id: ProjectId,
    pub name: String,
    pub colour: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl TryFrom<TeamMemberRecord> for TeamMember {
    type Error = DomainError;

    fn try_from(record: TeamMemberRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            id: record.id,
            project_id: record.project_id,
            name: validate_member_name(&record.name)?,
            colour: MemberColour::from_stored(record.colour)?,
            created_at: record.created_at,
            updated_at: record.updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{MAX_MEMBER_NAME_CHARS, MemberColour, TeamMember, display_labels, initials};
    use crate::{error::DomainError, ids::ProjectId};

    fn member(name: &str) -> TeamMember {
        TeamMember::new(ProjectId::new(), name, MemberColour::for_new_member(0)).unwrap()
    }

    #[test]
    fn initials_are_the_first_letter_of_the_first_two_words() {
        assert_eq!(initials("alex kim"), "AK");
        assert_eq!(initials("Mary Ann Smith"), "MA");
    }

    #[test]
    fn a_single_word_gives_its_first_two_letters() {
        assert_eq!(initials("Madonna"), "MA");
        assert_eq!(initials("X"), "X");
    }

    #[test]
    fn extra_spaces_do_not_make_empty_initials() {
        assert_eq!(initials("  Alex    Kim  "), "AK");
        assert_eq!(initials("   "), "");
    }

    #[test]
    fn non_latin_and_emoji_initials_stay_whole() {
        assert_eq!(initials("שלום כהן"), "שכ");
        assert_eq!(initials("🚀 launch"), "🚀L");
        assert_eq!(initials("e\u{301}cole"), "E\u{301}C");
        assert_eq!(initials("Émile Zola"), "ÉZ");
        assert_eq!(initials("田中太郎"), "田中");
        assert_eq!(initials("田中 太郎"), "田太");
    }

    #[test]
    fn names_are_trimmed_and_inner_spaces_are_joined() {
        assert_eq!(member("  Alex    Kim ").name(), "Alex Kim");
    }

    #[test]
    fn a_blank_name_is_refused() {
        assert_eq!(
            TeamMember::new(ProjectId::new(), "  ", MemberColour::for_new_member(0)).unwrap_err(),
            DomainError::EmptyName
        );
    }

    #[test]
    fn a_name_over_the_limit_is_refused() {
        let long = "x".repeat(MAX_MEMBER_NAME_CHARS + 1);
        assert_eq!(
            TeamMember::new(ProjectId::new(), &long, MemberColour::for_new_member(0)).unwrap_err(),
            DomainError::TooLong {
                max: MAX_MEMBER_NAME_CHARS
            }
        );
    }

    #[test]
    fn a_failed_rename_keeps_the_old_name() {
        let mut member = member("Dana");
        assert_eq!(member.rename("  ").unwrap_err(), DomainError::EmptyName);
        assert_eq!(member.name(), "Dana");
    }

    #[test]
    fn colours_cycle_through_the_palette_by_member_count() {
        let colours: Vec<usize> = (0..8)
            .map(|existing| MemberColour::for_new_member(existing).index())
            .collect();
        assert_eq!(colours, [0, 1, 2, 3, 4, 5, 0, 1]);
    }

    #[test]
    fn a_stored_colour_outside_the_palette_is_refused() {
        assert!(MemberColour::from_stored(5).is_ok());
        assert_eq!(
            MemberColour::from_stored(6).unwrap_err(),
            DomainError::UnknownColour(6)
        );
        assert!(MemberColour::from_stored(-1).is_err());
    }

    #[test]
    fn a_unique_name_has_no_suffix() {
        let members = [member("Dana"), member("Sam")];
        assert_eq!(display_labels(&members), ["Dana", "Sam"]);
    }

    #[test]
    fn same_named_members_are_numbered_in_list_order() {
        let members = [member("Alex Kim"), member("Dana"), member("Alex Kim")];
        assert_eq!(
            display_labels(&members),
            ["Alex Kim (1)", "Dana", "Alex Kim (2)"]
        );
    }

    #[test]
    fn names_that_differ_only_in_case_count_as_the_same() {
        let members = [member("alex kim"), member("Alex Kim")];
        assert_eq!(display_labels(&members), ["alex kim (1)", "Alex Kim (2)"]);
    }

    #[test]
    fn removing_a_duplicate_drops_the_suffix() {
        let members = [member("Alex Kim"), member("Alex Kim")];
        assert_eq!(display_labels(&members[..1]), ["Alex Kim"]);
    }
}
