//! Manual ordering. Each task's position is a string key, and keys sort byte by byte. A key can
//! always be found between two others, so moving a task rewrites only its own row.
//!
//! The alphabet is base 62 in ASCII order, so byte comparison and digit order agree. A key is a
//! fraction written after an implied `0.`. Keys never end in the smallest digit, `0`, because
//! `a0` and `a` are the same fraction, and only one spelling is kept.
//!
//! Pure logic only, so the rules are tested without a database.

use crate::{error::DomainError, ids::TaskId};

/// The alphabet, in ASCII order. Byte comparison of keys follows this order, so it must not change.
pub const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

/// A column whose keys grow past this many characters is renumbered in one transaction.
pub const REBALANCE_LENGTH: usize = 32;

/// The key of the first task in an empty column: the middle of the range.
pub const FIRST_KEY: &str = "V";

/// A key strictly between `before` and `after`. `None` means no bound on that side, so
/// `(None, None)` gives [`FIRST_KEY`], `(None, Some(first))` gives a key before `first`, and
/// `(Some(last), None)` gives a key after `last`.
pub fn key_between(before: Option<&str>, after: Option<&str>) -> Result<String, DomainError> {
    let lower = before.map(canonical).unwrap_or_default();
    let upper = after.map(canonical);
    validate(&lower)?;
    if let Some(upper) = &upper {
        validate(upper)?;
        if lower.as_str() >= upper.as_str() {
            return Err(DomainError::PositionsOutOfOrder);
        }
    }
    Ok(midpoint(&lower, upper.as_deref()))
}

/// The key for a task put at the start of a column whose first key is `first`.
pub fn before(first: Option<&str>) -> Result<String, DomainError> {
    key_between(None, first)
}

/// The key for a task put at the end of a column whose last key is `last`.
pub fn after(last: Option<&str>) -> Result<String, DomainError> {
    key_between(last, None)
}

/// Whether any of a column's keys is too long, so the column should be renumbered.
pub fn needs_rebalance(keys: &[&str]) -> bool {
    keys.iter().any(|key| key.len() > REBALANCE_LENGTH)
}

/// Evenly spaced keys for `ids`, in the order given. Every key is canonical, so the result can
/// be stored and compared directly.
pub fn rebalance(ids: &[TaskId]) -> Vec<(TaskId, String)> {
    let count = ids.len() as u128;
    // The smallest width that leaves a step of at least BASE between neighbouring keys.
    let mut width: usize = 1;
    while (BASE as u128).pow(width as u32) <= (count + 1) * BASE as u128 {
        width += 1;
    }
    let total = (BASE as u128).pow(width as u32);
    let step = total / (count + 1);
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            let value = step * (index as u128 + 1);
            (*id, canonical(&encode(value, width)))
        })
        .collect()
}

/// Drops trailing zeros. They don't change the fraction, and removing them keeps keys in their
/// canonical form.
fn canonical(key: &str) -> String {
    key.trim_end_matches('0').to_string()
}

fn validate(key: &str) -> Result<(), DomainError> {
    if key.bytes().all(|byte| DIGITS.contains(&byte)) {
        Ok(())
    } else {
        Err(DomainError::InvalidPositionKey {
            key: key.to_string(),
        })
    }
}

fn digit_value(byte: u8) -> usize {
    DIGITS
        .iter()
        .position(|&digit| digit == byte)
        .expect("keys are validated before use")
}

/// The digit at `index`, with the smallest digit past the end.
fn digit_at(key: &[u8], index: usize) -> u8 {
    key.get(index).copied().unwrap_or(DIGITS[0])
}

/// The midpoint of `lower` and `upper`. `None` means no upper bound. Needs `lower < upper`.
fn midpoint(lower: &str, upper: Option<&str>) -> String {
    if let Some(upper) = upper {
        // Shared leading digits stay as they are, and the midpoint goes after them.
        let shared = upper
            .bytes()
            .enumerate()
            .take_while(|&(index, byte)| digit_at(lower.as_bytes(), index) == byte)
            .count();
        if shared > 0 {
            let rest = lower.get(shared..).unwrap_or("");
            return format!(
                "{}{}",
                &upper[..shared],
                midpoint(rest, Some(&upper[shared..]))
            );
        }
    }

    let lower_digit = lower
        .as_bytes()
        .first()
        .map_or(0, |&byte| digit_value(byte));
    let upper_digit = upper.map_or(BASE, |upper| digit_value(upper.as_bytes()[0]));
    if upper_digit - lower_digit > 1 {
        // Round half up. The result is strictly between the two digits.
        let mid = (lower_digit + upper_digit).div_ceil(2);
        return (DIGITS[mid] as char).to_string();
    }
    // The first digits are neighbours. If the upper key is a single digit, the result extends the
    // lower key. Otherwise the upper key already starts above the lower key's first digit.
    if let Some(upper) = upper.filter(|upper| upper.len() > 1) {
        return upper[..1].to_string();
    }
    let rest = lower.get(1..).unwrap_or("");
    format!("{}{}", DIGITS[lower_digit] as char, midpoint(rest, None))
}

/// `value` written as exactly `width` base-62 digits.
fn encode(mut value: u128, width: usize) -> String {
    let mut digits = vec![DIGITS[0]; width];
    for slot in digits.iter_mut().rev() {
        *slot = DIGITS[(value % BASE as u128) as usize];
        value /= BASE as u128;
    }
    String::from_utf8(digits).expect("the alphabet is ASCII")
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use uuid::Uuid;

    use super::{
        FIRST_KEY, REBALANCE_LENGTH, after, before, key_between, needs_rebalance, rebalance,
    };
    use crate::{error::DomainError, ids::TaskId};

    fn id(n: u128) -> TaskId {
        TaskId::from_uuid(Uuid::from_u128(n))
    }

    fn between(lower: &str, upper: &str) -> String {
        let key = key_between(Some(lower), Some(upper)).unwrap();
        assert!(lower < key.as_str(), "{lower} < {key}");
        assert!(key.as_str() < upper, "{key} < {upper}");
        assert!(!key.ends_with('0'), "{key} ends in the smallest digit");
        key
    }

    #[test]
    fn an_empty_column_starts_in_the_middle() {
        assert_eq!(key_between(None, None).unwrap(), FIRST_KEY);
    }

    #[test]
    fn keys_go_before_the_first_and_after_the_last() {
        assert!(before(Some("V")).unwrap().as_str() < "V");
        assert!(after(Some("V")).unwrap().as_str() > "V");
    }

    #[test]
    fn a_key_between_two_single_characters() {
        assert_eq!(between("a", "b"), "aV");
        assert_eq!(between("0", "2"), "1");
    }

    #[test]
    fn a_key_between_keys_that_end_in_the_smallest_character() {
        // `a0` is the same fraction as `a`, so it's read as `a`, and the result never ends in `0`.
        assert_eq!(key_between(Some("a0"), Some("a1")).unwrap(), "a0V");
    }

    #[test]
    fn keys_with_shared_prefixes_keep_the_prefix() {
        assert!(between("ab", "ac").starts_with('a'));
    }

    #[test]
    fn the_result_is_always_strictly_between_its_bounds() {
        for (lower, upper) in [
            ("V", "W"),
            ("V", "zz"),
            ("zy", "zz"),
            ("a", "a1"),
            ("zzz", "zzzz"),
            ("0000001", "1"),
            ("Z", "a"),
        ] {
            between(lower, upper);
        }
        assert!(key_between(None, Some("0000001")).unwrap().as_str() < "0000001");
    }

    #[test]
    fn bounds_out_of_order_or_outside_the_alphabet_are_rejected() {
        assert_eq!(
            key_between(Some("b"), Some("a")),
            Err(DomainError::PositionsOutOfOrder)
        );
        assert_eq!(
            key_between(Some("a"), Some("a")),
            Err(DomainError::PositionsOutOfOrder)
        );
        assert!(matches!(
            key_between(Some("a-b"), None),
            Err(DomainError::InvalidPositionKey { .. })
        ));
    }

    #[test]
    fn repeated_inserts_into_the_same_gap_grow_until_a_rebalance_is_due() {
        let mut lower = "a".to_string();
        for _ in 0..200 {
            let key = between(&lower, "b");
            lower = key;
        }
        assert!(lower.len() > REBALANCE_LENGTH);
    }

    #[test]
    fn rebalance_spaces_keys_evenly_in_order() {
        let ids: Vec<TaskId> = (1..=5).map(id).collect();
        let keys = rebalance(&ids);
        assert_eq!(keys.iter().map(|(id, _)| *id).collect::<Vec<_>>(), ids);
        for pair in keys.windows(2) {
            assert!(pair[0].1 < pair[1].1);
        }
        assert!(keys.iter().all(|(_, key)| !key.ends_with('0')));
        let texts: Vec<&str> = keys.iter().map(|(_, key)| key.as_str()).collect();
        assert!(!needs_rebalance(&texts));
    }

    #[test]
    fn rebalancing_nothing_gives_nothing() {
        assert!(rebalance(&[]).is_empty());
    }

    #[test]
    fn a_long_key_asks_for_a_rebalance() {
        let long = "a".repeat(REBALANCE_LENGTH + 1);
        assert!(needs_rebalance(&[long.as_str()]));
        assert!(!needs_rebalance(&["a", "V"]));
    }

    /// A deterministic generator, so the random test is reproducible without a dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next_below(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % bound
        }
    }

    #[test]
    fn random_inserts_keep_a_sorted_order_matching_a_plain_list() {
        let mut random = Lcg(7);
        // The plain model: the ids in display order.
        let mut model: Vec<u128> = Vec::new();
        let mut keys: Vec<String> = Vec::new();
        for step in 0..3000u128 {
            let index = random.next_below(keys.len() + 1);
            let lower = index.checked_sub(1).map(|i| keys[i].clone());
            let upper = keys.get(index).cloned();
            let key = key_between(lower.as_deref(), upper.as_deref()).unwrap();
            keys.insert(index, key);
            model.insert(index, step);

            let mut sorted = keys.clone();
            sorted.sort();
            assert_eq!(sorted, keys, "step {step} broke the order");

            // Renumber when a key gets too long, as the write path does.
            let texts: Vec<&str> = keys.iter().map(String::as_str).collect();
            if needs_rebalance(&texts) {
                let ids: Vec<TaskId> = model.iter().map(|n| id(*n + 1)).collect();
                keys = rebalance(&ids).into_iter().map(|(_, key)| key).collect();
            }
        }
        assert_eq!(model.len(), keys.len());
    }
}
