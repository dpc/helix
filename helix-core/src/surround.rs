use std::fmt::Display;

use ropey::RopeSlice;

use crate::{
    graphemes::next_grapheme_boundary,
    match_brackets::{
        self, find_matching_bracket, find_matching_bracket_fuzzy, get_pair, BRACKETS,
    },
    movement::Direction,
    search, Range, Selection, Syntax,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    PairNotFound,
    CursorOverlap,
    RangeExceedsText,
    CursorOnAmbiguousPair,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match *self {
            Error::PairNotFound => "Surround pair not found around all cursors",
            Error::CursorOverlap => "Cursors overlap for a single surround pair range",
            Error::RangeExceedsText => "Cursor range exceeds text length",
            Error::CursorOnAmbiguousPair => "Cursor on ambiguous surround pair",
        })
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Finds the position of surround pairs of any [`crate::match_brackets::PAIRS`]
/// using tree-sitter when possible.
///
/// # Returns
///
/// Tuple `(anchor, head)`, meaning it is not always ordered.
pub fn find_nth_closest_pairs_pos(
    syntax: Option<&Syntax>,
    text: RopeSlice,
    range: Range,
    skip: usize,
) -> Result<(usize, usize)> {
    if skip == 0 {
        return Err(Error::PairNotFound);
    }
    match syntax {
        Some(syntax) => find_nth_closest_pairs_ts(syntax, text, range, skip),
        None => find_nth_closest_pairs_plain(text, range, skip),
    }
}

fn find_nth_closest_pairs_ts(
    syntax: &Syntax,
    text: RopeSlice,
    range: Range,
    mut skip: usize,
) -> Result<(usize, usize)> {
    let mut probe = range.to();
    let mut first = true;

    while skip > 0 {
        let mut closing = if first {
            first = false;
            match_brackets::find_matching_bracket_fuzzy_at_edge(Some(syntax), text, probe)
        } else {
            find_matching_bracket_fuzzy(syntax, text, probe)
        }
        .ok_or(Error::PairNotFound)?;
        let mut opening =
            find_matching_bracket(syntax, text, closing).ok_or(Error::PairNotFound)?;
        if closing < opening {
            (opening, closing) = (closing, opening);
        }

        let pair_end = next_grapheme_boundary(text, closing);
        if opening <= range.from() && range.to() <= pair_end {
            skip -= 1;
            if skip == 0 {
                return if let Direction::Forward = range.direction() {
                    Ok((opening, closing))
                } else {
                    Ok((closing, opening))
                };
            }
        }
        if probe == pair_end {
            return Err(Error::PairNotFound);
        }
        probe = pair_end;
    }

    Err(Error::PairNotFound)
}

fn find_nth_closest_pairs_plain(
    text: RopeSlice,
    range: Range,
    skip: usize,
) -> Result<(usize, usize)> {
    let mut open_positions: [Vec<usize>; BRACKETS.len()] = std::array::from_fn(|_| Vec::new());
    let mut candidates = Vec::new();
    for (pos, ch) in text.chars().enumerate() {
        if let Some(index) = BRACKETS.iter().position(|&(open, _)| open == ch) {
            open_positions[index].push(pos);
            continue;
        }
        let Some(index) = BRACKETS.iter().position(|&(_, close)| close == ch) else {
            continue;
        };
        let Some(open_pos) = open_positions[index].pop() else {
            continue;
        };
        if open_pos <= range.from() && range.to() <= next_grapheme_boundary(text, pos) {
            candidates.push((open_pos, pos));
        }
    }
    let key = |&(open, close): &(usize, usize)| {
        let affinity = if range.is_empty() && (open == range.head || close == range.head) {
            0
        } else if range.is_empty()
            && (next_grapheme_boundary(text, open) == range.head
                || next_grapheme_boundary(text, close) == range.head)
        {
            1
        } else {
            2
        };
        (affinity, close - open, open, close)
    };
    let base = *candidates
        .iter()
        .min_by_key(|pair| key(pair))
        .ok_or(Error::PairNotFound)?;
    let pair = if skip == 1 {
        base
    } else {
        let mut parents: Vec<_> = candidates
            .into_iter()
            .filter(|&pair| pair != base && pair.0 <= base.0 && base.1 <= pair.1)
            .collect();
        let index = skip - 2;
        if parents.len() <= index {
            return Err(Error::PairNotFound);
        }
        parents.select_nth_unstable_by_key(index, |&(open, close)| (close - open, open, close));
        parents[index]
    };
    match range.direction() {
        Direction::Forward => Ok(pair),
        Direction::Backward => Ok((pair.1, pair.0)),
    }
}

/// Find the position of surround pairs of `ch` which can be either a closing
/// or opening pair. `n` will skip n - 1 pairs (eg. n=2 will discard (only)
/// the first pair found and keep looking)
pub fn find_nth_pairs_pos(
    syntax: Option<&Syntax>,
    text: RopeSlice,
    ch: char,
    range: Range,
    n: usize,
) -> Result<(usize, usize)> {
    if text.len_chars() < 2 {
        return Err(Error::PairNotFound);
    }
    if text.len_chars() < range.to() {
        return Err(Error::RangeExceedsText);
    }

    let (open, close) = get_pair(ch);
    let pos = range.cursor(text);

    let (open, close) = if open == close {
        let mut adjacent = crate::selection::adjacent_char_positions(text, pos)
            .map_err(|_| Error::RangeExceedsText)?;
        if let Some(pair_pos) = adjacent.find(|&candidate| Some(open) == text.get_char(candidate)) {
            // Cursor is directly on match character for which the opening and closing pairs are the same. For instance: ", ', `
            //
            // This is potentially ambiguous, because there's no way to know which side of the char we should be searching on.
            let matching = syntax
                .and_then(|syntax| {
                    match_brackets::find_matching_bracket_fuzzy(syntax, text, pair_pos)
                })
                .filter(|&matching| identical_pair_encloses(text, pair_pos, matching, range))
                .or_else(|| matching_identical_delimiter_plaintext(text, pair_pos, open, range))
                .ok_or(Error::CursorOnAmbiguousPair)?;
            let mut pair = if pair_pos < matching {
                (pair_pos, matching)
            } else {
                (matching, pair_pos)
            };
            for _ in 1..n {
                let outer_open = search::find_nth_char(1, text, open, pair.0, Direction::Backward)
                    .ok_or(Error::PairNotFound)?;
                let outer_close = search::find_nth_char(
                    1,
                    text,
                    close,
                    next_grapheme_boundary(text, pair.1),
                    Direction::Forward,
                )
                .ok_or(Error::PairNotFound)?;
                pair = (outer_open, outer_close);
            }
            (Some(pair.0), Some(pair.1))
        } else {
            (
                search::find_nth_char(n, text, open, pos, Direction::Backward),
                search::find_nth_char(n, text, close, pos, Direction::Forward),
            )
        }
    } else {
        (
            find_nth_open_pair(text, open, close, pos, n),
            find_nth_close_pair(text, open, close, pos, n),
        )
    };

    // preserve original direction
    match range.direction() {
        Direction::Forward => Option::zip(open, close).ok_or(Error::PairNotFound),
        Direction::Backward => Option::zip(close, open).ok_or(Error::PairNotFound),
    }
}

fn matching_identical_delimiter_plaintext(
    text: RopeSlice,
    pos: usize,
    delimiter: char,
    range: Range,
) -> Option<usize> {
    let forward = text
        .chars_at(pos + 1)
        .position(|ch| ch == delimiter)
        .map(|offset| pos + 1 + offset);
    let backward = text
        .chars_at(pos)
        .reversed()
        .position(|ch| ch == delimiter)
        .map(|offset| pos - 1 - offset);
    forward
        .into_iter()
        .chain(backward)
        .find(|&matching| identical_pair_encloses(text, pos, matching, range))
}

fn identical_pair_encloses(text: RopeSlice, first: usize, second: usize, range: Range) -> bool {
    let (open, close) = if first < second {
        (first, second)
    } else {
        (second, first)
    };
    open <= range.from() && range.to() <= next_grapheme_boundary(text, close)
}

fn find_nth_open_pair(
    text: RopeSlice,
    open: char,
    close: char,
    mut pos: usize,
    n: usize,
) -> Option<usize> {
    if text.len_chars() < pos {
        return None;
    }
    if pos == text.len_chars() && 0 < pos && text.get_char(pos - 1) == Some(close) {
        pos -= 1;
    }

    let mut chars = text.chars_at(pos + 1);

    // Adjusts pos for the first iteration, and handles the case of the
    // cursor being *on* the close character which will get falsely stepped over
    // if not skipped here
    if chars.prev()? == open {
        return Some(pos);
    }

    for _ in 0..n {
        let mut step_over: usize = 0;

        loop {
            let c = chars.prev()?;
            pos = pos.saturating_sub(1);

            // ignore other surround pairs that are enclosed *within* our search scope
            if c == close {
                step_over += 1;
            } else if c == open {
                if step_over == 0 {
                    break;
                }

                step_over = step_over.saturating_sub(1);
            }
        }
    }

    Some(pos)
}

fn find_nth_close_pair(
    text: RopeSlice,
    open: char,
    close: char,
    mut pos: usize,
    n: usize,
) -> Option<usize> {
    if text.len_chars() < pos {
        return None;
    }
    if pos == text.len_chars() && 0 < pos && text.get_char(pos - 1) == Some(close) {
        return Some(pos - 1);
    }

    let mut chars = text.chars_at(pos);

    if chars.next()? == close {
        return Some(pos);
    }

    for _ in 0..n {
        let mut step_over: usize = 0;

        loop {
            let c = chars.next()?;
            pos += 1;

            if c == open {
                step_over += 1;
            } else if c == close {
                if step_over == 0 {
                    break;
                }

                step_over = step_over.saturating_sub(1);
            }
        }
    }

    Some(pos)
}

/// Find position of surround characters around every cursor. Returns None
/// if any positions overlap. Note that the positions are in a flat Vec.
/// Use get_surround_pos().chunks(2) to get matching pairs of surround positions.
/// `ch` can be either closing or opening pair. If `ch` is None, surround pairs
/// are automatically detected around each cursor (note that this may result
/// in them selecting different surround characters for each selection).
pub fn get_surround_pos(
    syntax: Option<&Syntax>,
    text: RopeSlice,
    selection: &Selection,
    ch: Option<char>,
    skip: usize,
) -> Result<Vec<usize>> {
    let mut change_pos = Vec::new();

    for &range in selection {
        let (open_pos, close_pos) = {
            let range_raw = match ch {
                Some(ch) => find_nth_pairs_pos(syntax, text, ch, range, skip)?,
                None => find_nth_closest_pairs_pos(syntax, text, range, skip)?,
            };
            let range = Range::new(range_raw.0, range_raw.1);
            (range.from(), range.to())
        };
        if change_pos.contains(&open_pos) || change_pos.contains(&close_pos) {
            return Err(Error::CursorOverlap);
        }
        // ensure the positions are always paired in the forward direction
        change_pos.extend_from_slice(&[open_pos.min(close_pos), close_pos.max(open_pos)]);
    }
    Ok(change_pos)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Range;

    use ropey::Rope;
    use smallvec::SmallVec;

    #[test]
    fn test_get_surround_pos() {
        #[rustfmt::skip]
        let (doc, selection, expectations) =
            rope_with_selections_and_expectations(
                "(some) (chars)\n(newline)",
                "_ ^  _ _ ^   _\n_    ^  _"
            );

        assert_eq!(
            get_surround_pos(None, doc.slice(..), &selection, Some('('), 1).unwrap(),
            expectations
        );
    }

    #[test]
    fn test_get_surround_pos_bail_different_surround_chars() {
        #[rustfmt::skip]
        let (doc, selection, _) =
            rope_with_selections_and_expectations(
                "[some]\n(chars)xx\n(newline)",
                "  ^   \n  ^      \n         "
            );

        assert_eq!(
            get_surround_pos(None, doc.slice(..), &selection, Some('('), 1),
            Err(Error::PairNotFound)
        );
    }

    #[test]
    fn test_get_surround_pos_bail_overlapping_surround_chars() {
        #[rustfmt::skip]
        let (doc, selection, _) =
            rope_with_selections_and_expectations(
                "[some]\n(chars)xx\n(newline)",
                "      \n       ^ \n      ^  "
            );

        assert_eq!(
            get_surround_pos(None, doc.slice(..), &selection, Some('('), 1),
            Err(Error::PairNotFound) // overlapping surround chars
        );
    }

    #[test]
    fn test_get_surround_pos_bail_cursor_overlap() {
        #[rustfmt::skip]
        let (doc, selection, _) =
            rope_with_selections_and_expectations(
                "[some]\n(chars)xx\n(newline)",
                "  ^^  \n         \n         "
            );

        assert_eq!(
            get_surround_pos(None, doc.slice(..), &selection, Some('['), 1),
            Err(Error::CursorOverlap)
        );
    }

    #[test]
    fn test_find_nth_pairs_pos_quote_success() {
        #[rustfmt::skip]
        let (doc, selection, expectations) =
            rope_with_selections_and_expectations(
                "some 'quoted text' on this 'line'\n'and this one'",
                "     _        ^  _               \n              "
            );

        assert_eq!(2, expectations.len());
        assert_eq!(
            find_nth_pairs_pos(None, doc.slice(..), '\'', selection.primary(), 1)
                .expect("find should succeed"),
            (expectations[0], expectations[1])
        )
    }

    #[test]
    fn test_find_nth_pairs_pos_nested_quote_success() {
        #[rustfmt::skip]
        let (doc, selection, expectations) =
            rope_with_selections_and_expectations(
                "some 'nested 'quoted' text' on this 'line'\n'and this one'",
                "     _           ^        _               \n              "
            );

        assert_eq!(2, expectations.len());
        assert_eq!(
            find_nth_pairs_pos(None, doc.slice(..), '\'', selection.primary(), 2)
                .expect("find should succeed"),
            (expectations[0], expectations[1])
        )
    }

    #[test]
    fn test_find_nth_pairs_pos_on_quote_uses_plaintext_parity() {
        #[rustfmt::skip]
        let (doc, selection, _) =
            rope_with_selections_and_expectations(
                "some 'nested 'quoted' text' on this 'line'\n'and this one'",
                "                    ^                     \n              "
            );

        assert_eq!(
            find_nth_pairs_pos(None, doc.slice(..), '\'', selection.primary(), 1),
            Ok((20, 26))
        );
    }

    #[test]
    fn test_find_nth_closest_pairs_pos_index_range_panic() {
        #[rustfmt::skip]
        let (doc, selection, _) =
            rope_with_selections_and_expectations(
                "(a)c)",
                "^^^^^"
            );

        assert_eq!(
            find_nth_closest_pairs_pos(None, doc.slice(..), selection.primary(), 1),
            Ok((0, 2))
        )
    }

    #[test]
    fn surround_discovery_uses_right_then_left_adjacency_at_eof() {
        let doc = Rope::from("(a)");
        assert_eq!(
            find_nth_pairs_pos(None, doc.slice(..), ')', Range::point(3), 1),
            Ok((0, 2))
        );
        assert_eq!(
            find_nth_closest_pairs_pos(None, doc.slice(..), Range::point(3), 1),
            Ok((0, 2))
        );

        let doc = Rope::from("「é」");
        assert_eq!(
            find_nth_pairs_pos(None, doc.slice(..), '」', Range::point(3), 1),
            Ok((0, 2))
        );
    }

    #[test]
    fn closest_plain_surround_stops_at_nearby_pair_and_counts_only_parents() {
        let mut source = String::from("(a)");
        source.extend(std::iter::repeat_n("}{", 50_000));
        let doc = Rope::from(source);
        assert_eq!(
            find_nth_closest_pairs_pos(None, doc.slice(..), Range::point(1), 1),
            Ok((0, 2))
        );
        assert_eq!(
            find_nth_closest_pairs_pos(None, doc.slice(..), Range::point(1), 0),
            Err(Error::PairNotFound)
        );

        let siblings = Rope::from("(a)(b)");
        assert_eq!(
            find_nth_closest_pairs_pos(None, siblings.slice(..), Range::point(3), 1),
            Ok((3, 5))
        );
        assert_eq!(
            find_nth_closest_pairs_pos(None, siblings.slice(..), Range::point(3), 2),
            Err(Error::PairNotFound)
        );

        let competing = Rope::from("(xxxxxxxx[])");
        assert_eq!(
            find_nth_closest_pairs_pos(None, competing.slice(..), Range::point(11), 1),
            Ok((0, 11))
        );

        let depth = 20_000;
        let source = format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
        let nested = Rope::from(source);
        assert_eq!(
            find_nth_closest_pairs_pos(None, nested.slice(..), Range::point(depth), depth),
            Ok((0, 2 * depth))
        );

        let malformed = Rope::from("({x)}");
        assert_eq!(
            find_nth_closest_pairs_pos(None, malformed.slice(..), Range::point(2), 2),
            Err(Error::PairNotFound)
        );
    }

    #[test]
    fn identical_delimiter_adjacency_uses_plaintext_parity() {
        let doc = Rope::from("'é'");
        assert_eq!(
            find_nth_pairs_pos(None, doc.slice(..), '\'', Range::point(0), 1),
            Ok((0, 2))
        );
        assert_eq!(
            find_nth_pairs_pos(None, doc.slice(..), '\'', Range::point(3), 1),
            Ok((0, 2))
        );

        let apostrophe = Rope::from("it's 'path'");
        assert_eq!(
            find_nth_pairs_pos(None, apostrophe.slice(..), '\'', Range::point(5), 1),
            Ok((5, 10))
        );
    }

    // Create a Rope and a matching Selection using a specification language.
    // ^ is a single-point selection.
    // _ is an expected index. These are returned as a Vec<usize> for use in assertions.
    fn rope_with_selections_and_expectations(
        text: &str,
        spec: &str,
    ) -> (Rope, Selection, Vec<usize>) {
        if text.len() != spec.len() {
            panic!("specification must match text length -- are newlines aligned?");
        }

        let rope = Rope::from(text);

        let selections: SmallVec<[Range; 1]> = spec
            .match_indices('^')
            .map(|(i, _)| Range::point(i))
            .collect();

        let expectations: Vec<usize> = spec.match_indices('_').map(|(i, _)| i).collect();

        (rope, Selection::new(selections, 0), expectations)
    }
}
