//! Find-as-you-type matching for the session list.
//!
//! Every whitespace-separated term must match (in order, case-insensitive)
//! somewhere in the haystack, so `auth hstry` finds "Refactor auth" in the
//! hstry repo. Contiguous runs and word starts score higher, and a term that
//! matches inside the title outranks one that only matches metadata.

/// Score `needle` as an ordered subsequence of `hay` (both already
/// lowercased). Returns the score and the matched char indices.
pub fn subsequence(hay: &[char], needle: &[char]) -> Option<(i64, Vec<usize>)> {
    if needle.is_empty() {
        return Some((0, Vec::new()));
    }
    // Prefer a contiguous occurrence when one exists: it is what users expect
    // to see highlighted and it should outrank a scattered match.
    if let Some(start) = find_contiguous(hay, needle) {
        let positions: Vec<usize> = (start..start + needle.len()).collect();
        let boundary = is_boundary(hay, start);
        let score = 10 * needle.len() as i64
            + if boundary { 15 } else { 0 }
            + if start == 0 { 10 } else { 0 };
        return Some((score, positions));
    }

    let mut positions = Vec::with_capacity(needle.len());
    let mut score = 0i64;
    let mut from = 0usize;
    let mut prev: Option<usize> = None;
    for &nc in needle {
        let pos = (from..hay.len()).find(|&i| hay[i] == nc)?;
        score += 1;
        if prev.is_some_and(|p| p + 1 == pos) {
            score += 4;
        }
        if is_boundary(hay, pos) {
            score += 5;
        }
        positions.push(pos);
        prev = Some(pos);
        from = pos + 1;
    }
    // A term scattered across a whole sentence ("auth" in "Add --json output
    // to the") is noise, not a match. Allow abbreviations ("rfctr" in
    // "refactor") but reject wide spreads.
    let span = positions.last().unwrap_or(&0) - positions.first().unwrap_or(&0);
    if span > needle.len() * 3 + 2 {
        return None;
    }
    score -= (span as i64 / 4).min(10);
    Some((score, positions))
}

fn find_contiguous(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

fn is_boundary(hay: &[char], pos: usize) -> bool {
    pos == 0
        || matches!(
            hay[pos - 1],
            ' ' | '/' | '-' | '_' | '.' | ':' | '(' | '[' | '"' | '\''
        )
}

/// A searchable session: title plus secondary text (repo, agent, first prompt).
pub struct Target<'a> {
    pub title: &'a str,
    pub secondary: &'a str,
}

/// Result of matching a whole query against one target.
pub struct Match {
    pub score: i64,
    /// Char indices in the title to highlight.
    pub title_positions: Vec<usize>,
}

/// Every query term must match the title or the secondary text.
pub fn match_query(query: &str, target: &Target<'_>) -> Option<Match> {
    let terms: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|t| t.to_lowercase().chars().collect())
        .collect();
    if terms.is_empty() {
        return Some(Match {
            score: 0,
            title_positions: Vec::new(),
        });
    }
    let title: Vec<char> = target.title.to_lowercase().chars().collect();
    let secondary: Vec<char> = target.secondary.to_lowercase().chars().collect();

    let mut score = 0i64;
    let mut title_positions = Vec::new();
    for term in &terms {
        let in_title = subsequence(&title, term);
        // Metadata is a bag of identifiers: abbreviations may skip letters
        // inside one word ("dstudio") but not jump across words.
        let in_secondary = subsequence(&secondary, term).filter(|(_, positions)| {
            let (first, last) = (positions[0], positions[positions.len() - 1]);
            !secondary[first..=last].iter().any(|c| c.is_whitespace())
        });
        match (in_title, in_secondary) {
            (Some((s, positions)), other) => {
                // Title hits are worth more, but a much better metadata hit wins.
                let title_score = s + 20;
                let secondary_score = other.map_or(i64::MIN, |(s, _)| s);
                if title_score >= secondary_score {
                    score += title_score;
                    title_positions.extend(positions);
                } else {
                    score += secondary_score;
                }
            }
            (None, Some((s, _))) => score += s,
            (None, None) => return None,
        }
    }
    title_positions.sort_unstable();
    title_positions.dedup();
    Some(Match {
        score,
        title_positions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target<'a>(title: &'a str, secondary: &'a str) -> Target<'a> {
        Target { title, secondary }
    }

    #[test]
    fn all_terms_must_match() {
        let t = target("Refactor auth middleware", "work/hstry claude-code");
        assert!(match_query("auth hstry", &t).is_some());
        assert!(match_query("auth billing", &t).is_none());
    }

    #[test]
    fn contiguous_beats_scattered() {
        let contiguous = match_query("auth", &target("auth middleware", "")).unwrap();
        let scattered = match_query("auth", &target("a utility to handle", "")).unwrap();
        assert!(contiguous.score > scattered.score);
    }

    #[test]
    fn wide_scatter_is_not_a_match() {
        assert!(
            match_query("auth", &target("Add --json output to the list command", "")).is_none()
        );
        assert!(match_query("rfctr", &target("refactor the loader", "")).is_some());
    }

    #[test]
    fn metadata_scatter_stays_in_one_word() {
        let t = target(
            "Fix flaky retry test",
            "work/hstry claude-code The retry test",
        );
        assert!(match_query("auth", &t).is_none());
        assert!(match_query("dstudio", &target("x", "work/design-studio")).is_some());
    }

    #[test]
    fn title_hits_are_highlighted() {
        let m = match_query("retry", &target("Fix flaky retry test", "")).unwrap();
        assert_eq!(m.title_positions, vec![10, 11, 12, 13, 14]);
    }

    #[test]
    fn metadata_only_match_has_no_title_highlight() {
        let m = match_query("hstry", &target("Fix flaky retry test", "work/hstry")).unwrap();
        assert!(m.title_positions.is_empty());
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(match_query("   ", &target("x", "")).is_some());
    }
}
