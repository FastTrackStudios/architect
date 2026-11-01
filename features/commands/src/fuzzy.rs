//! Fuzzy matching, the way a palette needs it: the query's characters in
//! order anywhere in the text, scored so that what a person means comes
//! first — a match at a word's start, a run of characters together, the
//! text's own start — and the matched characters reported, to light up.

/// A match: how good it is, and which characters of the text it used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i64,
    /// Indices into the text's `char`s.
    pub positions: Vec<usize>,
}

/// How much each kind of hit is worth.
const HIT: i64 = 16;
const WORD_START: i64 = 24;
const RUN: i64 = 20;
const TEXT_START: i64 = 12;
const CASE: i64 = 2;
const GAP: i64 = 1;

/// Whether `c` starts a word after `prev`: after a separator, or a
/// capital after a small letter (`camelCase`).
fn starts_word(prev: Option<char>, c: char) -> bool {
    prev.is_none_or(|p| {
        !p.is_alphanumeric()
            || (p.is_lowercase() && c.is_uppercase())
            || (p.is_alphabetic() && c.is_numeric())
    })
}

/// Matches `query` in `text`, ignoring case and spaces in the query.
/// `None` when a character of the query is not there in order. An empty
/// query matches everything with score 0.
#[must_use]
pub fn fuzzy(query: &str, text: &str) -> Option<Match> {
    let wanted: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    if wanted.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }
    let chars: Vec<char> = text.chars().collect();
    // Greedy from the left, but preferring a word start when one is near:
    // for each wanted character, take the next word-start hit if there is
    // one before the next plain hit's word ends, else the next hit.
    let mut positions = Vec::with_capacity(wanted.len());
    let mut from = 0usize;
    for w in &wanted {
        let lower = w.to_lowercase().next().unwrap_or(*w);
        let hit = |i: &usize| {
            chars
                .get(*i)
                .is_some_and(|c| c.to_lowercase().next() == Some(lower))
        };
        let next = (from..chars.len()).find(|i| hit(i))?;
        let word = (next..chars.len()).find(|i| {
            hit(i)
                && starts_word(
                    i.checked_sub(1).and_then(|p| chars.get(p).copied()),
                    chars.get(*i).copied().unwrap_or(' '),
                )
        });
        // A word-start hit wins over a plain one unless the plain one
        // continues a run.
        let continues = positions
            .last()
            .is_some_and(|l: &usize| l.saturating_add(1) == next);
        let at = match word {
            Some(w) if !continues => w,
            _ => next,
        };
        positions.push(at);
        from = at.saturating_add(1);
    }
    Some(Match {
        score: score(&wanted, &chars, &positions),
        positions,
    })
}

fn score(wanted: &[char], chars: &[char], positions: &[usize]) -> i64 {
    let mut total = 0i64;
    let mut last: Option<usize> = None;
    for (w, &at) in wanted.iter().zip(positions) {
        let c = chars.get(at).copied().unwrap_or(' ');
        let prev = at.checked_sub(1).and_then(|p| chars.get(p).copied());
        total = total.saturating_add(HIT);
        if at == 0 {
            total = total.saturating_add(TEXT_START);
        }
        if starts_word(prev, c) {
            total = total.saturating_add(WORD_START);
        }
        if *w == c {
            total = total.saturating_add(CASE);
        }
        match last {
            Some(l) if l.saturating_add(1) == at => total = total.saturating_add(RUN),
            Some(l) => {
                let gap = i64::try_from(at.saturating_sub(l)).unwrap_or(i64::MAX);
                total = total.saturating_sub(gap.saturating_mul(GAP));
            }
            None => {
                let lead = i64::try_from(at).unwrap_or(i64::MAX);
                total = total.saturating_sub(lead.saturating_mul(GAP));
            }
        }
        last = Some(at);
    }
    // Shorter texts win a tie: "Close" over "Close all other tabs".
    total.saturating_sub(i64::try_from(chars.len()).unwrap_or(i64::MAX) / 8)
}
