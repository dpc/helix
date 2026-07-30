//! Selections are the primary editing construct. Even cursors are
//! defined as a selection range.
//!
//! All positioning is done via `char` offsets into the buffer.
//!
//! This implementation uses edge-based (beam cursor) semantics where positions
//! represent gaps between characters rather than the characters themselves.
//! This allows for zero-width selections (cursor position only) like in GUI editors.
use crate::{
    graphemes::{
        ensure_grapheme_boundary_next, ensure_grapheme_boundary_prev, prev_grapheme_boundary,
    },
    line_ending::get_line_ending,
    movement::Direction,
    tree_sitter::Node,
    Assoc, ChangeSet, RopeSlice,
};
use helix_stdx::range::is_subset;
use helix_stdx::rope::{self, RopeSliceExt};
use smallvec::{smallvec, SmallVec};
use std::{borrow::Cow, iter, slice};

/// Reports a selection endpoint outside the document's scalar-edge domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionBoundsError {
    /// The invalid character edge.
    pub position: usize,
    /// The document's valid EOF edge.
    pub len_chars: usize,
}

/// Returns the character with right affinity at `edge`.
///
/// EOF is a valid edge with no right-hand character. Out-of-bounds edges are
/// rejected rather than clamped.
pub fn char_at_edge(text: RopeSlice, edge: usize) -> Result<Option<char>, SelectionBoundsError> {
    if text.len_chars() < edge {
        return Err(SelectionBoundsError {
            position: edge,
            len_chars: text.len_chars(),
        });
    }
    Ok(text.get_char(edge))
}

/// Returns the byte range owned by an edge with right affinity.
///
/// A non-EOF edge owns the scalar immediately to its right. EOF owns the
/// natural empty byte range at the end of the document.
pub fn byte_range_at_edge(
    text: RopeSlice,
    edge: usize,
) -> Result<std::ops::Range<usize>, SelectionBoundsError> {
    char_at_edge(text, edge)?;
    Ok(text.char_to_byte(edge)..text.char_to_byte((edge + 1).min(text.len_chars())))
}

/// Returns adjacency candidates at an edge, checking right before left.
///
/// This helper is reserved for commands such as brace and surround discovery
/// whose semantics explicitly inspect both sides of the beam.
pub fn adjacent_char_positions(
    text: RopeSlice,
    edge: usize,
) -> Result<impl Iterator<Item = usize>, SelectionBoundsError> {
    char_at_edge(text, edge)?;
    Ok([
        Some(edge).filter(|&pos| pos < text.len_chars()),
        edge.checked_sub(1),
    ]
    .into_iter()
    .flatten())
}

/// A single selection range.
///
/// A range consists of an "anchor" and "head" position in
/// the text.  The head is the part that the user moves when
/// directly extending a selection.  The head and anchor
/// can be in any order, or even share the same position.
///
/// The anchor and head positions use gap indexing, meaning
/// that their indices represent the gaps *between* `char`s
/// rather than the `char`s themselves. For example, 1
/// represents the position between the first and second `char`.
///
/// Below are some examples of `Range` configurations.
/// The anchor and head indices are shown as "(anchor, head)"
/// tuples, followed by example text with "|" representing
/// the cursor (head) position and "[" "]" for selection bounds:
///
/// - (0, 3): `[Som|]e text` - selection from 0 to 3, cursor at 3.
/// - (3, 0): `|[Som]e text` - selection from 0 to 3, cursor at 0.
/// - (2, 7): `So[me te|]xt` - selection from 2 to 7, cursor at 7.
/// - (1, 1): `S|ome text` - cursor at position 1, no selection.
/// - (0, 0): `|Some text` - cursor at start, no selection.
///
/// Ranges are considered to be inclusive on the left and
/// exclusive on the right, regardless of anchor-head ordering.
/// This means, for example, that non-zero-width ranges that
/// are directly adjacent, sharing an edge, do not overlap.
/// However, a zero-width range will overlap with the shared
/// left-edge of another range.
///
/// This implementation uses beam cursor (edge-based) semantics where
/// the cursor position represents the gap between characters, allowing
/// for zero-width selections like in GUI text editors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// The anchor of the range: the side that doesn't move when extending.
    pub anchor: usize,
    /// The head of the range, moved when extending.
    pub head: usize,
    /// The previous visual offset (softwrapped lines and columns) from
    /// the start of the line
    pub old_visual_position: Option<(u32, u32)>,
}

impl Range {
    pub fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            old_visual_position: None,
        }
    }

    pub fn point(head: usize) -> Self {
        Self::new(head, head)
    }

    pub fn from_node(node: Node, text: RopeSlice, direction: Direction) -> Self {
        let from = text.byte_to_char(node.start_byte() as usize);
        let to = text.byte_to_char(node.end_byte() as usize);
        Range::new(from, to).with_direction(direction)
    }

    /// Start of the range.
    #[inline]
    #[must_use]
    pub fn from(&self) -> usize {
        std::cmp::min(self.anchor, self.head)
    }

    /// End of the range.
    #[inline]
    #[must_use]
    pub fn to(&self) -> usize {
        std::cmp::max(self.anchor, self.head)
    }

    /// Total length of the range.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.to() - self.from()
    }

    /// The (inclusive) range of lines that the range overlaps.
    #[inline]
    #[must_use]
    pub fn line_range(&self, text: RopeSlice) -> (usize, usize) {
        let from = self.from();
        let to = if self.is_empty() {
            self.to()
        } else {
            prev_grapheme_boundary(text, self.to()).max(from)
        };

        (text.char_to_line(from), text.char_to_line(to))
    }

    /// `true` when head and anchor are at the same position.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// `Direction::Backward` when head < anchor.
    /// `Direction::Forward` otherwise.
    #[inline]
    #[must_use]
    pub fn direction(&self) -> Direction {
        if self.head < self.anchor {
            Direction::Backward
        } else {
            Direction::Forward
        }
    }

    /// Flips the direction of the selection
    pub fn flip(&self) -> Self {
        Self {
            anchor: self.head,
            head: self.anchor,
            old_visual_position: self.old_visual_position,
        }
    }

    /// Returns the selection if it goes in the direction of `direction`,
    /// flipping the selection otherwise.
    pub fn with_direction(self, direction: Direction) -> Self {
        if self.direction() == direction {
            self
        } else {
            self.flip()
        }
    }

    /// Check two ranges for overlap.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        // To my eye, it's non-obvious why this works, but I arrived
        // at it after transforming the slower version that explicitly
        // enumerated more cases.  The unit tests are thorough.
        self.from() == other.from() || (self.to() > other.from() && other.to() > self.from())
    }

    #[inline]
    pub fn contains_range(&self, other: &Self) -> bool {
        self.from() <= other.from() && self.to() >= other.to()
    }

    pub fn contains(&self, pos: usize) -> bool {
        self.from() <= pos && pos < self.to()
    }

    /// Map a range through a set of changes. Returns a new range representing
    /// the same position after the changes are applied. Note that this
    /// function runs in O(N) (N is number of changes) and can therefore
    /// cause performance problems if run for a large number of ranges as the
    /// complexity is then O(MN) (for multicuror M=N usually). Instead use
    /// [Selection::map] or [ChangeSet::update_positions].
    pub fn map(mut self, changes: &ChangeSet) -> Self {
        use std::cmp::Ordering;
        if changes.is_empty() {
            return self;
        }
        let unequal_replacements = UnequalReplacements::new(changes);

        let positions_to_map = match self.anchor.cmp(&self.head) {
            Ordering::Equal => [
                (&mut self.anchor, Assoc::AfterSticky),
                (&mut self.head, Assoc::AfterSticky),
            ],
            Ordering::Less => {
                let lower = endpoint_assoc(&unequal_replacements, self.anchor, EndpointRole::Lower);
                let upper = endpoint_assoc(&unequal_replacements, self.head, EndpointRole::Upper);
                [(&mut self.anchor, lower), (&mut self.head, upper)]
            }
            Ordering::Greater => {
                let lower = endpoint_assoc(&unequal_replacements, self.head, EndpointRole::Lower);
                let upper = endpoint_assoc(&unequal_replacements, self.anchor, EndpointRole::Upper);
                [(&mut self.head, lower), (&mut self.anchor, upper)]
            }
        };
        changes.update_positions(positions_to_map.into_iter());
        self.old_visual_position = None;
        self
    }

    /// Extend the range to cover at least `from` `to`.
    #[must_use]
    pub fn extend(&self, from: usize, to: usize) -> Self {
        debug_assert!(from <= to);

        if self.anchor <= self.head {
            Self {
                anchor: self.anchor.min(from),
                head: self.head.max(to),
                old_visual_position: None,
            }
        } else {
            Self {
                anchor: self.anchor.max(to),
                head: self.head.min(from),
                old_visual_position: None,
            }
        }
    }

    /// Returns a range that encompasses both input ranges.
    ///
    /// This is like `extend()`, but tries to negotiate the
    /// anchor/head ordering between the two input ranges.
    #[must_use]
    pub fn merge(&self, other: Self) -> Self {
        if self.anchor > self.head && other.anchor > other.head {
            Range {
                anchor: self.anchor.max(other.anchor),
                head: self.head.min(other.head),
                old_visual_position: None,
            }
        } else {
            Range {
                anchor: self.from().min(other.from()),
                head: self.to().max(other.to()),
                old_visual_position: None,
            }
        }
    }

    // groupAt

    /// Returns the text inside this range given the text of the whole buffer.
    ///
    /// The returned `Cow` is a reference if the range of text is inside a single
    /// chunk of the rope. Otherwise a copy of the text is returned. Consider
    /// using `slice` instead if you do not need a `Cow` or `String` to avoid copying.
    #[inline]
    pub fn fragment<'a, 'b: 'a>(&'a self, text: RopeSlice<'b>) -> Cow<'b, str> {
        self.slice(text).into()
    }

    /// Returns the text inside this range given the text of the whole buffer.
    ///
    /// The returned value is a reference to the passed slice. This method never
    /// copies any contents.
    #[inline]
    pub fn slice<'a, 'b: 'a>(&'a self, text: RopeSlice<'b>) -> RopeSlice<'b> {
        text.slice(self.from()..self.to())
    }

    //--------------------------------
    // Alignment methods.

    /// Compute a possibly new range from this range, with its ends
    /// shifted as needed to align with grapheme boundaries.
    ///
    /// Zero-width ranges will always stay zero-width, and non-zero-width
    /// ranges will never collapse to zero-width.
    ///
    /// # Panics
    ///
    /// Panics when either endpoint is beyond `slice`'s EOF edge. Use
    /// [`Selection::try_ensure_invariants`] for fallible external input.
    #[must_use]
    pub fn grapheme_aligned(&self, slice: RopeSlice) -> Self {
        use std::cmp::Ordering;
        assert!(self.anchor <= slice.len_chars() && self.head <= slice.len_chars());
        let (new_anchor, new_head) = match self.anchor.cmp(&self.head) {
            Ordering::Equal => {
                let pos = ensure_grapheme_boundary_prev(slice, self.anchor);
                (pos, pos)
            }
            Ordering::Less => (
                ensure_grapheme_boundary_prev(slice, self.anchor),
                ensure_grapheme_boundary_next(slice, self.head),
            ),
            Ordering::Greater => (
                ensure_grapheme_boundary_next(slice, self.anchor),
                ensure_grapheme_boundary_prev(slice, self.head),
            ),
        };
        Range {
            anchor: new_anchor,
            head: new_head,
            old_visual_position: if new_anchor == self.anchor {
                self.old_visual_position
            } else {
                None
            },
        }
    }

    //--------------------------------
    // Cursor methods (beam/edge-based).

    /// Gets the cursor position (the head of the range).
    ///
    /// With beam cursor semantics, the cursor is simply at the head position,
    /// representing the edge between characters.
    #[must_use]
    #[inline]
    pub fn cursor(self, _text: RopeSlice) -> usize {
        self.head
    }

    /// Puts the cursor at `char_idx`, optionally extending the selection.
    ///
    /// With beam cursor semantics, this simply moves the head to the new position.
    /// If extending, the anchor stays in place; otherwise both anchor and head
    /// move to the new position (creating a zero-width selection/cursor).
    ///
    /// This method assumes that `char_idx` is already properly grapheme-aligned.
    #[must_use]
    #[inline]
    pub fn put_cursor(self, _text: RopeSlice, char_idx: usize, extend: bool) -> Range {
        if extend {
            Range::new(self.anchor, char_idx)
        } else {
            Range::point(char_idx)
        }
    }

    /// The line number that the cursor is on.
    #[inline]
    #[must_use]
    pub fn cursor_line(&self, text: RopeSlice) -> usize {
        text.char_to_line(self.cursor(text))
    }

    /// Returns true if this Range covers a single grapheme in the given text
    pub fn is_single_grapheme(&self, doc: RopeSlice) -> bool {
        let mut graphemes = doc.slice(self.from()..self.to()).graphemes();
        let first = graphemes.next();
        let second = graphemes.next();
        first.is_some() && second.is_none()
    }

    /// Converts this char range into an in order byte range, discarding
    /// direction.
    pub fn into_byte_range(&self, text: RopeSlice) -> (usize, usize) {
        (text.char_to_byte(self.from()), text.char_to_byte(self.to()))
    }
}

#[derive(Clone, Copy)]
enum EndpointRole {
    Lower,
    Upper,
}

struct UnequalReplacements(Vec<(usize, usize)>);

impl UnequalReplacements {
    fn new(changes: &ChangeSet) -> Self {
        Self(
            changes
                .changes_iter()
                .filter_map(|(from, to, replacement)| {
                    let inserted_len = replacement.map_or(0, |text| text.chars().count());
                    (to - from != inserted_len).then_some((from, to))
                })
                .collect(),
        )
    }

    fn contains(&self, position: usize) -> bool {
        let index = self.0.partition_point(|&(from, _)| from < position);
        index != 0 && position < self.0[index - 1].1
    }
}

fn endpoint_assoc(
    unequal_replacements: &UnequalReplacements,
    position: usize,
    role: EndpointRole,
) -> Assoc {
    let inside_unequal_replacement = unequal_replacements.contains(position);

    match (role, inside_unequal_replacement) {
        (EndpointRole::Lower, false) | (EndpointRole::Upper, true) => Assoc::AfterSticky,
        (EndpointRole::Upper, false) | (EndpointRole::Lower, true) => Assoc::BeforeSticky,
    }
}

impl From<(usize, usize)> for Range {
    fn from((anchor, head): (usize, usize)) -> Self {
        Self {
            anchor,
            head,
            old_visual_position: None,
        }
    }
}

impl From<Range> for helix_stdx::Range {
    fn from(range: Range) -> Self {
        Self {
            start: range.from(),
            end: range.to(),
        }
    }
}

/// A selection consists of one or more selection ranges.
/// invariant: A selection can never be empty (always contains at least primary range).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    ranges: SmallVec<[Range; 1]>,
    primary_index: usize,
}

/// A restricted selection whose ranges have not been aligned or normalized.
///
/// Transaction builders use this type to preserve every scalar participant
/// until phase-1 mapping completes.
pub struct UnalignedSelection(pub(crate) Selection);

impl UnalignedSelection {
    /// Returns the logical primary participant before alignment.
    pub fn primary_index(&self) -> usize {
        self.0.primary_index()
    }

    /// Maps scalar endpoints without alignment or collision normalization.
    ///
    /// The returned transient selection must not be stored directly. Align it
    /// against post-change text and run exactly one D10 normalization first.
    pub fn map_no_normalize(self, changes: &ChangeSet) -> Selection {
        self.0.map_no_normalize(changes)
    }
}

#[allow(clippy::len_without_is_empty)] // a Selection is never empty
impl Selection {
    // eq

    #[inline]
    #[must_use]
    pub fn primary(&self) -> Range {
        self.ranges[self.primary_index]
    }

    #[inline]
    #[must_use]
    pub fn primary_mut(&mut self) -> &mut Range {
        &mut self.ranges[self.primary_index]
    }

    /// Ensure selection containing only the primary selection.
    pub fn into_single(self) -> Self {
        if self.ranges.len() == 1 {
            self
        } else {
            Self {
                ranges: smallvec![self.ranges[self.primary_index]],
                primary_index: 0,
            }
        }
    }

    /// Adds a new range to the selection and makes it the primary range.
    pub fn push(mut self, range: Range) -> Self {
        self.ranges.push(range);
        self.set_primary_index(self.ranges().len() - 1);
        self.normalize()
    }

    /// Removes a range from the selection.
    pub fn remove(mut self, index: usize) -> Self {
        assert!(
            self.ranges.len() > 1,
            "can't remove the last range from a selection!"
        );

        self.ranges.remove(index);
        if index < self.primary_index || self.primary_index == self.ranges.len() {
            self.primary_index -= 1;
        }
        self
    }

    /// Replace a range in the selection with a new range.
    pub fn replace(mut self, index: usize, range: Range) -> Self {
        self.ranges[index] = range;
        self.normalize()
    }

    /// Maps scalar endpoints, aligns them against the post-change text, then normalizes once.
    ///
    /// `new_text` must be the document text after `changes` has been applied.
    pub fn map(self, changes: &ChangeSet, new_text: RopeSlice) -> Self {
        let mut selection = self.map_no_normalize(changes);
        for range in &mut selection.ranges {
            *range = range.grapheme_aligned(new_text);
        }
        selection.normalize()
    }

    /// Map selections over a set of changes. Useful for adjusting the selection position after
    /// applying changes to a document. Doesn't normalize the selection
    /// Performs only phase 1 of edit mapping.
    ///
    /// The result may be unaligned or overlapping and must not be stored
    /// directly. Callers must align against post-change text and then apply
    /// exactly one D10 normalization.
    pub fn map_no_normalize(mut self, changes: &ChangeSet) -> Self {
        if changes.is_empty() {
            return self;
        }
        let unequal_replacements = UnequalReplacements::new(changes);

        let positions_to_map = self.ranges.iter_mut().flat_map(|range| {
            use std::cmp::Ordering;
            range.old_visual_position = None;
            match range.anchor.cmp(&range.head) {
                Ordering::Equal => [
                    (&mut range.anchor, Assoc::AfterSticky),
                    (&mut range.head, Assoc::AfterSticky),
                ],
                Ordering::Less => {
                    let lower =
                        endpoint_assoc(&unequal_replacements, range.anchor, EndpointRole::Lower);
                    let upper =
                        endpoint_assoc(&unequal_replacements, range.head, EndpointRole::Upper);
                    [(&mut range.anchor, lower), (&mut range.head, upper)]
                }
                Ordering::Greater => {
                    let lower =
                        endpoint_assoc(&unequal_replacements, range.head, EndpointRole::Lower);
                    let upper =
                        endpoint_assoc(&unequal_replacements, range.anchor, EndpointRole::Upper);
                    [(&mut range.head, lower), (&mut range.anchor, upper)]
                }
            }
        });
        changes.update_positions(positions_to_map);
        self
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    /// Returns an iterator over the line ranges of each range in the selection.
    ///
    /// Adjacent and overlapping line ranges of the [Range]s in the selection are merged.
    pub fn line_ranges<'a>(&'a self, text: RopeSlice<'a>) -> LineRangeIter<'a> {
        LineRangeIter {
            ranges: self.ranges.iter().peekable(),
            text,
        }
    }

    pub fn range_bounds(&self) -> impl Iterator<Item = helix_stdx::Range> + '_ {
        self.ranges.iter().map(|&range| range.into())
    }

    pub fn primary_index(&self) -> usize {
        self.primary_index
    }

    pub fn set_primary_index(&mut self, idx: usize) {
        assert!(idx < self.ranges.len());
        self.primary_index = idx;
    }

    #[must_use]
    /// Constructs a selection holding a single range.
    pub fn single(anchor: usize, head: usize) -> Self {
        Self {
            ranges: smallvec![Range {
                anchor,
                head,
                old_visual_position: None
            }],
            primary_index: 0,
        }
    }

    /// Constructs a selection holding a single cursor.
    pub fn point(pos: usize) -> Self {
        Self::single(pos, pos)
    }

    /// Normalizes a `Selection`.
    ///
    /// Ranges are sorted by [Range::from], with overlapping ranges merged.
    fn normalize(mut self) -> Self {
        if self.len() < 2 {
            return self;
        }
        let mut ranges: Vec<_> = self
            .ranges
            .into_iter()
            .enumerate()
            .map(|(index, range)| (range, index == self.primary_index))
            .collect();
        ranges.sort_unstable_by(|(left, _), (right, _)| {
            left.from()
                .cmp(&right.from())
                .then_with(|| right.to().cmp(&left.to()))
                .then_with(|| {
                    matches!(left.direction(), Direction::Backward)
                        .cmp(&matches!(right.direction(), Direction::Backward))
                })
        });

        let mut normalized: SmallVec<[Range; 1]> = SmallVec::new();
        let mut primary_index = None;
        let mut index = 0;
        while index < ranges.len() {
            let (winner, mut contains_primary) = ranges[index];
            let mut from = winner.from();
            let mut to = winner.to();
            let mut direction = winner.direction();
            index += 1;

            while index < ranges.len() && Range::new(from, to).overlaps(&ranges[index].0) {
                let (range, is_primary) = ranges[index];
                from = from.min(range.from());
                to = to.max(range.to());
                if is_primary {
                    direction = range.direction();
                }
                contains_primary |= is_primary;
                index += 1;
            }

            let range = Range::new(from, to).with_direction(direction);
            if contains_primary {
                primary_index = Some(normalized.len());
            }
            normalized.push(range);
        }

        self.ranges = normalized;
        self.primary_index = primary_index.expect("the primary range must survive normalization");
        self
    }

    /// Replaces ranges with one spanning from first to last range.
    pub fn merge_ranges(self) -> Self {
        let first = self.ranges.first().unwrap();
        let last = self.ranges.last().unwrap();
        Selection::new(smallvec![first.merge(*last)], 0)
    }

    /// Merges all ranges that are consecutive.
    pub fn merge_consecutive_ranges(mut self) -> Self {
        let mut primary = self.ranges[self.primary_index];

        self.ranges.dedup_by(|curr_range, prev_range| {
            if prev_range.to() == curr_range.from() {
                let new_range = curr_range.merge(*prev_range);
                if prev_range == &primary || curr_range == &primary {
                    primary = new_range;
                }
                *prev_range = new_range;
                true
            } else {
                false
            }
        });

        self.primary_index = self
            .ranges
            .iter()
            .position(|&range| range == primary)
            .unwrap();

        self
    }

    /// Constructs and normalizes a selection from grapheme-aligned ranges.
    #[must_use]
    pub fn new(ranges: SmallVec<[Range; 1]>, primary_index: usize) -> Self {
        assert!(!ranges.is_empty());
        assert!(primary_index < ranges.len());
        Self::new_unaligned(ranges, primary_index).normalize()
    }

    /// Validates, aligns, and D10-normalizes scalar ranges against `text`.
    pub fn try_new(
        ranges: SmallVec<[Range; 1]>,
        primary_index: usize,
        text: RopeSlice,
    ) -> Result<Self, SelectionBoundsError> {
        Self::new_unaligned(ranges, primary_index).try_ensure_invariants(text)
    }

    /// Constructs an unnormalized selection for a transaction result.
    ///
    /// Ranges must remain distinct until they are aligned against post-change
    /// text. Callers must align and normalize the result before passing it to
    /// consumers that assume stored-selection invariants.
    pub(crate) fn new_unaligned(ranges: SmallVec<[Range; 1]>, primary_index: usize) -> Self {
        assert!(!ranges.is_empty());
        assert!(primary_index < ranges.len());
        Self {
            ranges,
            primary_index,
        }
    }

    /// Maps each range and normalizes the results.
    pub fn transform<F>(mut self, mut f: F) -> Self
    where
        F: FnMut(Range) -> Range,
    {
        for range in self.ranges.iter_mut() {
            *range = f(*range)
        }
        self.normalize()
    }

    /// Maps each range to multiple ranges and normalizes the results.
    pub fn transform_iter<F, I>(mut self, f: F) -> Self
    where
        F: FnMut(Range) -> I,
        I: Iterator<Item = Range>,
    {
        self.ranges = self.ranges.into_iter().flat_map(f).collect();
        self.normalize()
    }

    /// Ensures the selection adheres to the following invariants:
    ///
    /// 1. All ranges are grapheme aligned.
    /// 2. Ranges are non-overlapping.
    /// 3. Ranges are sorted by their position in the text.
    ///
    /// With beam cursor semantics, zero-width ranges (cursor only) are valid.
    ///
    /// # Panics
    ///
    /// Panics when an endpoint is beyond EOF. Fallible callers must use
    /// [`Selection::try_ensure_invariants`] instead.
    pub fn ensure_invariants(self, text: RopeSlice) -> Self {
        self.try_ensure_invariants(text)
            .expect("selection endpoints must be within document bounds")
    }

    /// Validates endpoint bounds, aligns graphemes, and applies D10 once.
    pub fn try_ensure_invariants(mut self, text: RopeSlice) -> Result<Self, SelectionBoundsError> {
        let len_chars = text.len_chars();
        for range in &self.ranges {
            for position in [range.anchor, range.head] {
                if len_chars < position {
                    return Err(SelectionBoundsError {
                        position,
                        len_chars,
                    });
                }
            }
        }
        for range in &mut self.ranges {
            *range = range.grapheme_aligned(text);
        }
        Ok(self.normalize())
    }

    /// Transforms the selection into cursor positions (head positions).
    pub fn cursors(self, text: RopeSlice) -> Self {
        self.transform(|range| Range::point(range.cursor(text)))
            .normalize()
    }

    pub fn fragments<'a>(
        &'a self,
        text: RopeSlice<'a>,
    ) -> impl DoubleEndedIterator<Item = Cow<'a, str>> + ExactSizeIterator<Item = Cow<'a, str>>
    {
        self.ranges.iter().map(move |range| range.fragment(text))
    }

    pub fn slices<'a>(
        &'a self,
        text: RopeSlice<'a>,
    ) -> impl DoubleEndedIterator<Item = RopeSlice<'a>> + ExactSizeIterator<Item = RopeSlice<'a>> + 'a
    {
        self.ranges.iter().map(move |range| range.slice(text))
    }

    #[inline(always)]
    pub fn iter(&self) -> std::slice::Iter<'_, Range> {
        self.ranges.iter()
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// returns true if self ⊇ other
    pub fn contains(&self, other: &Selection) -> bool {
        is_subset::<true>(self.range_bounds(), other.range_bounds())
    }
}

impl<'a> IntoIterator for &'a Selection {
    type Item = &'a Range;
    type IntoIter = std::slice::Iter<'a, Range>;

    fn into_iter(self) -> std::slice::Iter<'a, Range> {
        self.ranges().iter()
    }
}

impl IntoIterator for Selection {
    type Item = Range;
    type IntoIter = smallvec::IntoIter<[Range; 1]>;

    fn into_iter(self) -> smallvec::IntoIter<[Range; 1]> {
        self.ranges.into_iter()
    }
}

impl FromIterator<Range> for Selection {
    fn from_iter<T: IntoIterator<Item = Range>>(ranges: T) -> Self {
        Self::new(ranges.into_iter().collect(), 0)
    }
}

impl From<Range> for Selection {
    fn from(range: Range) -> Self {
        Self {
            ranges: smallvec![range],
            primary_index: 0,
        }
    }
}

pub struct LineRangeIter<'a> {
    ranges: iter::Peekable<slice::Iter<'a, Range>>,
    text: RopeSlice<'a>,
}

impl Iterator for LineRangeIter<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let (start, mut end) = self.ranges.next()?.line_range(self.text);
        while let Some((next_start, next_end)) =
            self.ranges.peek().map(|range| range.line_range(self.text))
        {
            // Merge overlapping and adjacent ranges.
            // This subtraction cannot underflow because the ranges are sorted.
            if next_start - end <= 1 {
                end = next_end;
                self.ranges.next();
            } else {
                break;
            }
        }

        Some((start, end))
    }
}

// TODO: checkSelection -> check if valid for doc length && sorted

pub fn keep_or_remove_matches(
    text: RopeSlice,
    selection: &Selection,
    regex: &rope::Regex,
    remove: bool,
) -> Option<Selection> {
    let result: SmallVec<_> = selection
        .iter()
        .filter(|range| regex.is_match(text.regex_input_at(range.from()..range.to())) ^ remove)
        .copied()
        .collect();

    // TODO: figure out a new primary index
    if !result.is_empty() {
        return Some(Selection::new(result, 0));
    }
    None
}

// TODO: support to split on capture #N instead of whole match
pub fn select_on_matches(
    text: RopeSlice,
    selection: &Selection,
    regex: &rope::Regex,
) -> Option<Selection> {
    let mut result = SmallVec::with_capacity(selection.len());

    for sel in selection {
        for mat in regex.find_iter(text.regex_input_at(sel.from()..sel.to())) {
            // TODO: retain range direction

            let start = text.byte_to_char(mat.start());
            let end = text.byte_to_char(mat.end());

            let range = Range::new(start, end);
            // Make sure the match is not right outside of the selection.
            // These invalid matches can come from using RegEx anchors like `^`, `$`
            if range != Range::point(sel.to()) {
                result.push(range);
            }
        }
    }

    // TODO: figure out a new primary index
    if !result.is_empty() {
        return Some(Selection::new(result, 0));
    }

    None
}

pub fn split_on_newline(text: RopeSlice, selection: &Selection) -> Selection {
    let mut result = SmallVec::with_capacity(selection.len());

    for sel in selection {
        // Special case: zero-width selection.
        if sel.from() == sel.to() {
            result.push(*sel);
            continue;
        }

        let sel_start = sel.from();
        let sel_end = sel.to();

        let mut start = sel_start;

        for line in sel.slice(text).lines() {
            let Some(line_ending) = get_line_ending(&line) else {
                break;
            };
            let line_end = start + line.len_chars();
            // TODO: retain range direction
            result.push(Range::new(start, line_end - line_ending.len_chars()));
            start = line_end;
        }

        if start < sel_end {
            result.push(Range::new(start, sel_end));
        }
    }

    // TODO: figure out a new primary index
    Selection::new(result, 0)
}

pub fn split_on_matches(text: RopeSlice, selection: &Selection, regex: &rope::Regex) -> Selection {
    let mut result = SmallVec::with_capacity(selection.len());

    for sel in selection {
        // Special case: zero-width selection.
        if sel.from() == sel.to() {
            result.push(*sel);
            continue;
        }

        let sel_start = sel.from();
        let sel_end = sel.to();
        let mut start = sel_start;

        for mat in regex.find_iter(text.regex_input_at(sel_start..sel_end)) {
            // TODO: retain range direction
            let end = text.byte_to_char(mat.start());
            result.push(Range::new(start, end));
            start = text.byte_to_char(mat.end());
        }

        if start < sel_end {
            result.push(Range::new(start, sel_end));
        }
    }

    // TODO: figure out a new primary index
    Selection::new(result, 0)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Rope, Tendril, Transaction};

    #[test]
    #[should_panic]
    fn test_new_empty() {
        let _ = Selection::new(smallvec![], 0);
    }

    #[test]
    fn test_create_normalizes_and_merges() {
        let sel = Selection::new(
            smallvec![
                Range::new(10, 12),
                Range::new(6, 7),
                Range::new(4, 5),
                Range::new(3, 4),
                Range::new(0, 6),
                Range::new(7, 8),
                Range::new(9, 13),
                Range::new(13, 14),
            ],
            0,
        )
        .normalize();

        let res = sel
            .ranges
            .into_iter()
            .map(|range| format!("{}/{}", range.anchor, range.head))
            .collect::<Vec<String>>()
            .join(",");

        assert_eq!(res, "0/6,6/7,7/8,9/13,13/14");

        // it correctly calculates a new primary index
        let sel = Selection::new(
            smallvec![Range::new(0, 2), Range::new(1, 5), Range::new(4, 7)],
            2,
        )
        .normalize();

        let res = sel
            .ranges
            .into_iter()
            .map(|range| format!("{}/{}", range.anchor, range.head))
            .collect::<Vec<String>>()
            .join(",");

        assert_eq!(res, "0/7");
        assert_eq!(sel.primary_index, 0);
    }

    #[test]
    fn test_create_merges_adjacent_points() {
        let sel = Selection::new(
            smallvec![
                Range::new(10, 12),
                Range::new(12, 12),
                Range::new(12, 12),
                Range::new(10, 10),
                Range::new(8, 10),
            ],
            0,
        )
        .normalize();

        let res = sel
            .ranges
            .into_iter()
            .map(|range| format!("{}/{}", range.anchor, range.head))
            .collect::<Vec<String>>()
            .join(",");

        assert_eq!(res, "8/10,10/12,12/12");
    }

    #[test]
    fn test_contains() {
        let range = Range::new(10, 12);

        assert!(!range.contains(9));
        assert!(range.contains(10));
        assert!(range.contains(11));
        assert!(!range.contains(12));
        assert!(!range.contains(13));

        let range = Range::new(9, 6);
        assert!(!range.contains(9));
        assert!(range.contains(7));
        assert!(range.contains(6));
    }

    #[test]
    fn test_overlaps() {
        fn overlaps(a: (usize, usize), b: (usize, usize)) -> bool {
            Range::new(a.0, a.1).overlaps(&Range::new(b.0, b.1))
        }

        // Two non-zero-width ranges, no overlap.
        assert!(!overlaps((0, 3), (3, 6)));
        assert!(!overlaps((0, 3), (6, 3)));
        assert!(!overlaps((3, 0), (3, 6)));
        assert!(!overlaps((3, 0), (6, 3)));
        assert!(!overlaps((3, 6), (0, 3)));
        assert!(!overlaps((3, 6), (3, 0)));
        assert!(!overlaps((6, 3), (0, 3)));
        assert!(!overlaps((6, 3), (3, 0)));

        // Two non-zero-width ranges, overlap.
        assert!(overlaps((0, 4), (3, 6)));
        assert!(overlaps((0, 4), (6, 3)));
        assert!(overlaps((4, 0), (3, 6)));
        assert!(overlaps((4, 0), (6, 3)));
        assert!(overlaps((3, 6), (0, 4)));
        assert!(overlaps((3, 6), (4, 0)));
        assert!(overlaps((6, 3), (0, 4)));
        assert!(overlaps((6, 3), (4, 0)));

        // Zero-width and non-zero-width range, no overlap.
        assert!(!overlaps((0, 3), (3, 3)));
        assert!(!overlaps((3, 0), (3, 3)));
        assert!(!overlaps((3, 3), (0, 3)));
        assert!(!overlaps((3, 3), (3, 0)));

        // Zero-width and non-zero-width range, overlap.
        assert!(overlaps((1, 4), (1, 1)));
        assert!(overlaps((4, 1), (1, 1)));
        assert!(overlaps((1, 1), (1, 4)));
        assert!(overlaps((1, 1), (4, 1)));

        assert!(overlaps((1, 4), (3, 3)));
        assert!(overlaps((4, 1), (3, 3)));
        assert!(overlaps((3, 3), (1, 4)));
        assert!(overlaps((3, 3), (4, 1)));

        // Two zero-width ranges, no overlap.
        assert!(!overlaps((0, 0), (1, 1)));
        assert!(!overlaps((1, 1), (0, 0)));

        // Two zero-width ranges, overlap.
        assert!(overlaps((1, 1), (1, 1)));
    }

    #[test]
    fn test_grapheme_aligned() {
        let r = Rope::from_str("\r\nHi\r\n");
        let s = r.slice(..);

        // Zero-width.
        assert_eq!(Range::new(0, 0).grapheme_aligned(s), Range::new(0, 0));
        assert_eq!(Range::new(1, 1).grapheme_aligned(s), Range::new(0, 0));
        assert_eq!(Range::new(2, 2).grapheme_aligned(s), Range::new(2, 2));
        assert_eq!(Range::new(3, 3).grapheme_aligned(s), Range::new(3, 3));
        assert_eq!(Range::new(4, 4).grapheme_aligned(s), Range::new(4, 4));
        assert_eq!(Range::new(5, 5).grapheme_aligned(s), Range::new(4, 4));
        assert_eq!(Range::new(6, 6).grapheme_aligned(s), Range::new(6, 6));

        // Forward.
        assert_eq!(Range::new(0, 1).grapheme_aligned(s), Range::new(0, 2));
        assert_eq!(Range::new(1, 2).grapheme_aligned(s), Range::new(0, 2));
        assert_eq!(Range::new(2, 3).grapheme_aligned(s), Range::new(2, 3));
        assert_eq!(Range::new(3, 4).grapheme_aligned(s), Range::new(3, 4));
        assert_eq!(Range::new(4, 5).grapheme_aligned(s), Range::new(4, 6));
        assert_eq!(Range::new(5, 6).grapheme_aligned(s), Range::new(4, 6));

        assert_eq!(Range::new(0, 2).grapheme_aligned(s), Range::new(0, 2));
        assert_eq!(Range::new(1, 3).grapheme_aligned(s), Range::new(0, 3));
        assert_eq!(Range::new(2, 4).grapheme_aligned(s), Range::new(2, 4));
        assert_eq!(Range::new(3, 5).grapheme_aligned(s), Range::new(3, 6));
        assert_eq!(Range::new(4, 6).grapheme_aligned(s), Range::new(4, 6));

        // Reverse.
        assert_eq!(Range::new(1, 0).grapheme_aligned(s), Range::new(2, 0));
        assert_eq!(Range::new(2, 1).grapheme_aligned(s), Range::new(2, 0));
        assert_eq!(Range::new(3, 2).grapheme_aligned(s), Range::new(3, 2));
        assert_eq!(Range::new(4, 3).grapheme_aligned(s), Range::new(4, 3));
        assert_eq!(Range::new(5, 4).grapheme_aligned(s), Range::new(6, 4));
        assert_eq!(Range::new(6, 5).grapheme_aligned(s), Range::new(6, 4));

        assert_eq!(Range::new(2, 0).grapheme_aligned(s), Range::new(2, 0));
        assert_eq!(Range::new(3, 1).grapheme_aligned(s), Range::new(3, 0));
        assert_eq!(Range::new(4, 2).grapheme_aligned(s), Range::new(4, 2));
        assert_eq!(Range::new(5, 3).grapheme_aligned(s), Range::new(6, 3));
        assert_eq!(Range::new(6, 4).grapheme_aligned(s), Range::new(6, 4));
    }

    #[test]
    fn test_select_on_matches() {
        let r = Rope::from_str("Nobody expects the Spanish inquisition");
        let s = r.slice(..);

        let selection = Selection::single(0, r.len_chars());
        assert_eq!(
            select_on_matches(s, &selection, &rope::Regex::new(r"[A-Z][a-z]*").unwrap()),
            Some(Selection::new(
                smallvec![Range::new(0, 6), Range::new(19, 26)],
                0
            ))
        );

        let r = Rope::from_str("This\nString\n\ncontains multiple\nlines");
        let s = r.slice(..);

        let start_of_line = rope::RegexBuilder::new()
            .syntax(rope::Config::new().multi_line(true))
            .build(r"^")
            .unwrap();
        let end_of_line = rope::RegexBuilder::new()
            .syntax(rope::Config::new().multi_line(true))
            .build(r"$")
            .unwrap();

        // line without ending
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 4), &start_of_line),
            Some(Selection::single(0, 0))
        );
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 4), &end_of_line),
            None
        );
        // line with ending
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 5), &start_of_line),
            Some(Selection::single(0, 0))
        );
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 5), &end_of_line),
            Some(Selection::single(4, 4))
        );
        // line with start of next line
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 6), &start_of_line),
            Some(Selection::new(
                smallvec![Range::point(0), Range::point(5)],
                0
            ))
        );
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 6), &end_of_line),
            Some(Selection::single(4, 4))
        );

        // multiple lines
        assert_eq!(
            select_on_matches(
                s,
                &Selection::single(0, s.len_chars()),
                &rope::RegexBuilder::new()
                    .syntax(rope::Config::new().multi_line(true))
                    .build(r"^[a-z ]*$")
                    .unwrap()
            ),
            Some(Selection::new(
                smallvec![Range::point(12), Range::new(13, 30), Range::new(31, 36)],
                0
            ))
        );
    }

    #[test]
    fn test_select_on_matches_crlf() {
        let r = Rope::from_str("This\r\nString\r\n\r\ncontains multiple\r\nlines");
        let s = r.slice(..);

        let start_of_line = rope::RegexBuilder::new()
            .syntax(rope::Config::new().multi_line(true).crlf(true))
            .build(r"^")
            .unwrap();
        let end_of_line = rope::RegexBuilder::new()
            .syntax(rope::Config::new().multi_line(true).crlf(true))
            .build(r"$")
            .unwrap();

        // line without ending
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 4), &start_of_line),
            Some(Selection::single(0, 0))
        );
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 4), &end_of_line),
            None
        );
        // line with ending
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 5), &start_of_line),
            Some(Selection::single(0, 0))
        );
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 5), &end_of_line),
            Some(Selection::single(4, 4))
        );
        // line with start of next line
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 7), &start_of_line),
            Some(Selection::new(
                smallvec![Range::point(0), Range::point(6)],
                0
            ))
        );
        assert_eq!(
            select_on_matches(s, &Selection::single(0, 6), &end_of_line),
            Some(Selection::single(4, 4))
        );

        // multiple lines
        assert_eq!(
            select_on_matches(
                s,
                &Selection::single(0, s.len_chars()),
                &rope::RegexBuilder::new()
                    .syntax(rope::Config::new().multi_line(true).crlf(true))
                    .build(r"^[a-z ]*$")
                    .unwrap()
            ),
            Some(Selection::new(
                smallvec![Range::point(14), Range::new(16, 33), Range::new(35, 40)],
                0
            ))
        );
    }

    #[test]
    fn test_line_range() {
        let r = Rope::from_str("\r\nHi\r\nthere!");
        let s = r.slice(..);

        // Zero-width ranges.
        assert_eq!(Range::new(0, 0).line_range(s), (0, 0));
        assert_eq!(Range::new(1, 1).line_range(s), (0, 0));
        assert_eq!(Range::new(2, 2).line_range(s), (1, 1));
        assert_eq!(Range::new(3, 3).line_range(s), (1, 1));

        // Forward ranges.
        assert_eq!(Range::new(0, 1).line_range(s), (0, 0));
        assert_eq!(Range::new(0, 2).line_range(s), (0, 0));
        assert_eq!(Range::new(0, 3).line_range(s), (0, 1));
        assert_eq!(Range::new(1, 2).line_range(s), (0, 0));
        assert_eq!(Range::new(2, 3).line_range(s), (1, 1));
        assert_eq!(Range::new(3, 8).line_range(s), (1, 2));
        assert_eq!(Range::new(0, 12).line_range(s), (0, 2));

        // Reverse ranges.
        assert_eq!(Range::new(1, 0).line_range(s), (0, 0));
        assert_eq!(Range::new(2, 0).line_range(s), (0, 0));
        assert_eq!(Range::new(3, 0).line_range(s), (0, 1));
        assert_eq!(Range::new(2, 1).line_range(s), (0, 0));
        assert_eq!(Range::new(3, 2).line_range(s), (1, 1));
        assert_eq!(Range::new(8, 3).line_range(s), (1, 2));
        assert_eq!(Range::new(12, 0).line_range(s), (0, 2));
    }

    #[test]
    fn selection_line_ranges() {
        let (text, selection) = crate::test::print(
            r#"                                           L0
            #[|these]# line #(|ranges)# are #(|merged)#   L1
                                                          L2
            single one-line #(|range)#                    L3
                                                          L4
            single #(|multiline                           L5
            range)#                                       L6
                                                          L7
            these #(|multiline                            L8
            ranges)# are #(|also                          L9
            merged)#                                      L10
                                                          L11
            adjacent #(|ranges)#                          L12
            are merged #(|the same way)#                  L13
            "#,
        );
        let rope = Rope::from_str(&text);
        assert_eq!(
            vec![(1, 1), (3, 3), (5, 6), (8, 10), (12, 13)],
            selection.line_ranges(rope.slice(..)).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn test_cursor() {
        let r = Rope::from_str("\r\nHi\r\nthere!");
        let s = r.slice(..);

        // With beam cursor semantics, cursor() simply returns head.
        // Zero-width ranges.
        assert_eq!(Range::new(0, 0).cursor(s), 0);
        assert_eq!(Range::new(2, 2).cursor(s), 2);
        assert_eq!(Range::new(3, 3).cursor(s), 3);

        // Forward ranges - cursor is at head.
        assert_eq!(Range::new(0, 2).cursor(s), 2);
        assert_eq!(Range::new(0, 3).cursor(s), 3);
        assert_eq!(Range::new(3, 6).cursor(s), 6);

        // Reverse ranges - cursor is at head.
        assert_eq!(Range::new(2, 0).cursor(s), 0);
        assert_eq!(Range::new(6, 2).cursor(s), 2);
        assert_eq!(Range::new(6, 3).cursor(s), 3);
    }

    #[test]
    fn test_put_cursor() {
        let r = Rope::from_str("\r\nHi\r\nthere!");
        let s = r.slice(..);

        // With beam cursor semantics, put_cursor simply moves head to the new position.
        // When extending, anchor stays in place; otherwise we get a zero-width range.

        // Zero-width ranges - extending from anchor.
        assert_eq!(Range::new(0, 0).put_cursor(s, 0, true), Range::new(0, 0));
        assert_eq!(Range::new(0, 0).put_cursor(s, 2, true), Range::new(0, 2));
        assert_eq!(Range::new(2, 3).put_cursor(s, 4, true), Range::new(2, 4));
        assert_eq!(Range::new(2, 8).put_cursor(s, 4, true), Range::new(2, 4));
        assert_eq!(Range::new(8, 8).put_cursor(s, 4, true), Range::new(8, 4));

        // Forward ranges - anchor stays, head moves.
        assert_eq!(Range::new(3, 6).put_cursor(s, 0, true), Range::new(3, 0));
        assert_eq!(Range::new(3, 6).put_cursor(s, 2, true), Range::new(3, 2));
        assert_eq!(Range::new(3, 6).put_cursor(s, 3, true), Range::new(3, 3));
        assert_eq!(Range::new(3, 6).put_cursor(s, 4, true), Range::new(3, 4));
        assert_eq!(Range::new(3, 6).put_cursor(s, 6, true), Range::new(3, 6));
        assert_eq!(Range::new(3, 6).put_cursor(s, 8, true), Range::new(3, 8));

        // Reverse ranges - anchor stays, head moves.
        assert_eq!(Range::new(6, 3).put_cursor(s, 0, true), Range::new(6, 0));
        assert_eq!(Range::new(6, 3).put_cursor(s, 2, true), Range::new(6, 2));
        assert_eq!(Range::new(6, 3).put_cursor(s, 3, true), Range::new(6, 3));
        assert_eq!(Range::new(6, 3).put_cursor(s, 4, true), Range::new(6, 4));
        assert_eq!(Range::new(6, 3).put_cursor(s, 6, true), Range::new(6, 6));
        assert_eq!(Range::new(6, 3).put_cursor(s, 8, true), Range::new(6, 8));

        // Non-extending - creates zero-width range at new position.
        assert_eq!(Range::new(3, 6).put_cursor(s, 0, false), Range::new(0, 0));
        assert_eq!(Range::new(6, 3).put_cursor(s, 4, false), Range::new(4, 4));
    }

    #[test]
    fn test_split_on_matches() {
        let text = Rope::from(" abcd efg wrs   xyz 123 456");

        let selection = Selection::new(smallvec![Range::new(0, 9), Range::new(11, 20),], 0);

        let result = split_on_matches(
            text.slice(..),
            &selection,
            &rope::Regex::new(r"\s+").unwrap(),
        );

        assert_eq!(
            result.ranges(),
            &[
                // TODO: rather than this behavior, maybe we want it
                // to be based on which side is the anchor?
                //
                // We get a leading zero-width range when there's
                // a leading match because ranges are inclusive on
                // the left.  Imagine, for example, if the entire
                // selection range were matched: you'd still want
                // at least one range to remain after the split.
                Range::new(0, 0),
                Range::new(1, 5),
                Range::new(6, 9),
                Range::new(11, 13),
                Range::new(16, 19),
                // In contrast to the comment above, there is no
                // _trailing_ zero-width range despite the trailing
                // match, because ranges are exclusive on the right.
            ]
        );

        assert_eq!(
            result.fragments(text.slice(..)).collect::<Vec<_>>(),
            &["", "abcd", "efg", "rs", "xyz"]
        );
    }

    #[test]
    fn test_merge_consecutive_ranges() {
        let selection = Selection::new(
            smallvec![
                Range::new(0, 1),
                Range::new(1, 10),
                Range::new(15, 20),
                Range::new(25, 26),
                Range::new(26, 30)
            ],
            4,
        );

        let result = selection.merge_consecutive_ranges();

        assert_eq!(
            result.ranges(),
            &[Range::new(0, 10), Range::new(15, 20), Range::new(25, 30)]
        );
        assert_eq!(result.primary_index, 2);

        let selection = Selection::new(smallvec![Range::new(0, 1)], 0);
        let result = selection.merge_consecutive_ranges();

        assert_eq!(result.ranges(), &[Range::new(0, 1)]);
        assert_eq!(result.primary_index, 0);

        let selection = Selection::new(
            smallvec![
                Range::new(0, 1),
                Range::new(1, 5),
                Range::new(5, 8),
                Range::new(8, 10),
                Range::new(10, 15),
                Range::new(18, 25)
            ],
            3,
        );

        let result = selection.merge_consecutive_ranges();

        assert_eq!(result.ranges(), &[Range::new(0, 15), Range::new(18, 25)]);
        assert_eq!(result.primary_index, 0);
    }

    #[test]
    fn test_selection_contains() {
        fn contains(a: Vec<(usize, usize)>, b: Vec<(usize, usize)>) -> bool {
            let sela = Selection::new(a.iter().map(|a| Range::new(a.0, a.1)).collect(), 0);
            let selb = Selection::new(b.iter().map(|b| Range::new(b.0, b.1)).collect(), 0);
            sela.contains(&selb)
        }

        // exact match
        assert!(contains(vec!((1, 1)), vec!((1, 1))));

        // larger set contains smaller
        assert!(contains(vec!((1, 1), (2, 2), (3, 3)), vec!((2, 2))));

        // multiple matches
        assert!(contains(vec!((1, 1), (2, 2)), vec!((1, 1), (2, 2))));

        // smaller set can't contain bigger
        assert!(!contains(vec!((1, 1)), vec!((1, 1), (2, 2))));

        assert!(contains(
            vec!((1, 1), (2, 4), (5, 6), (7, 9), (10, 13)),
            vec!((3, 4), (7, 9))
        ));
        assert!(!contains(vec!((1, 1), (5, 6)), vec!((1, 6))));

        // multiple ranges of other are all contained in some ranges of self,
        assert!(contains(
            vec!((1, 4), (7, 10)),
            vec!((1, 2), (3, 4), (7, 9))
        ));
    }

    #[test]
    fn test_backward_secondary_selection_preserved() {
        // Test that backward selections (head < anchor) are preserved
        // through ensure_invariants and other operations
        let text = Rope::from_str("Hello world!");
        let s = text.slice(..);

        // Create a selection with a forward primary and backward secondary
        // Primary: positions 0-5 (forward), Secondary: positions 6-11 (backward: anchor=11, head=6)
        let selection = Selection::new(smallvec![Range::new(0, 5), Range::new(11, 6)], 0);

        // Verify the secondary selection is backward
        let secondary = &selection.ranges()[1];
        assert_eq!(secondary.anchor, 11);
        assert_eq!(secondary.head, 6);
        assert!(
            secondary.head < secondary.anchor,
            "Secondary should be backward"
        );

        // Verify from() and to() return correct values regardless of direction
        assert_eq!(secondary.from(), 6);
        assert_eq!(secondary.to(), 11);

        // Ensure invariants should NOT change the direction
        let normalized = selection.clone().ensure_invariants(s);

        // Check that ranges still exist and have correct from/to
        // Note: after ensure_invariants the order might change due to sorting by from()
        let ranges = normalized.ranges();
        assert_eq!(ranges.len(), 2, "Should still have 2 ranges");

        // Find the range that covers 6-11
        let backward_range = ranges.iter().find(|r| r.from() == 6 && r.to() == 11);
        assert!(
            backward_range.is_some(),
            "Should still have range covering 6-11"
        );
        let backward_range = backward_range.unwrap();

        // The direction should be preserved
        assert_eq!(backward_range.anchor, 11, "Anchor should be 11");
        assert_eq!(backward_range.head, 6, "Head should be 6");
        assert!(
            backward_range.head < backward_range.anchor,
            "Backward direction should be preserved after ensure_invariants"
        );
    }

    #[test]
    fn validation_rejects_out_of_bounds_without_clamping() {
        let text = Rope::from("abc");
        let error = Selection::single(0, 4)
            .try_ensure_invariants(text.slice(..))
            .unwrap_err();
        assert_eq!(
            error,
            SelectionBoundsError {
                position: 4,
                len_chars: 3
            }
        );
    }

    #[test]
    fn construction_preserves_participants_until_grapheme_alignment() {
        let text = Rope::from("a\u{301}b");
        let selection = Selection::new_unaligned(smallvec![Range::point(0), Range::new(1, 2)], 1);
        assert_eq!(selection.ranges(), &[Range::point(0), Range::new(1, 2)]);

        let normalized = selection.ensure_invariants(text.slice(..));
        assert_eq!(normalized.ranges(), &[Range::new(0, 2)]);
        assert_eq!(normalized.primary_index(), 0);
    }

    #[test]
    fn grapheme_alignment_covers_points_ranges_crlf_and_eof() {
        let text = Rope::from("a\u{301}\r\n🙂");
        let slice = text.slice(..);
        for (input, expected) in [
            (Range::point(1), Range::point(0)),
            (Range::new(1, 2), Range::new(0, 2)),
            (Range::new(2, 3), Range::new(2, 4)),
            (Range::new(3, 2), Range::new(4, 2)),
            (Range::point(5), Range::point(5)),
        ] {
            assert_eq!(input.grapheme_aligned(slice), expected);
        }
    }

    #[test]
    fn d10_normalization_is_permutation_independent() {
        let primary = Range::new(7, 2);
        let ranges = [primary, Range::new(1, 4), Range::new(6, 9)];
        for permutation in [
            [ranges[0], ranges[1], ranges[2]],
            [ranges[0], ranges[2], ranges[1]],
            [ranges[1], ranges[0], ranges[2]],
            [ranges[1], ranges[2], ranges[0]],
            [ranges[2], ranges[0], ranges[1]],
            [ranges[2], ranges[1], ranges[0]],
        ] {
            let primary_index = permutation
                .iter()
                .position(|&range| range == primary)
                .unwrap();
            let selection =
                Selection::new(permutation.into_iter().collect(), primary_index).normalize();
            assert_eq!(selection.ranges(), &[Range::new(9, 1)]);
            assert_eq!(selection.primary_index(), 0);
        }
    }

    #[test]
    fn d10_secondary_merge_uses_deterministic_winner() {
        let selection = Selection::new(
            smallvec![
                Range::point(20),
                Range::new(5, 2),
                Range::new(1, 4),
                Range::new(3, 6)
            ],
            0,
        )
        .normalize();
        assert_eq!(selection.ranges(), &[Range::new(1, 6), Range::point(20)]);
        assert_eq!(selection.primary_index(), 1);
    }

    #[test]
    fn d10_secondary_ties_choose_widest_then_forward() {
        let widest = Selection::new(
            smallvec![
                Range::point(20),
                Range::new(4, 1),
                Range::new(6, 1),
                Range::new(2, 5)
            ],
            0,
        )
        .normalize();
        assert_eq!(widest.ranges()[0], Range::new(6, 1));

        let forward = Selection::new(
            smallvec![Range::point(20), Range::new(5, 1), Range::new(1, 5)],
            0,
        )
        .normalize();
        assert_eq!(forward.ranges()[0], Range::new(1, 5));
    }

    #[test]
    fn mapping_applies_scalar_align_and_d10_phases_in_order() {
        struct MappingCase {
            name: &'static str,
            range: Range,
            change: (usize, usize, Option<Tendril>),
            scalar: Range,
            aligned: Range,
            final_range: Range,
        }

        fn phases(
            source: &str,
            range: Range,
            change: (usize, usize, Option<Tendril>),
        ) -> (Range, Range, Range) {
            let old_text = Rope::from(source);
            let transaction = Transaction::change(&old_text, std::iter::once(change));
            let scalar = Selection::from(range)
                .map_no_normalize(transaction.changes())
                .primary();
            let mut new_text = old_text.clone();
            assert!(transaction.changes().apply(&mut new_text));
            let aligned = scalar.grapheme_aligned(new_text.slice(..));
            let final_range = Selection::from(range)
                .map(transaction.changes(), new_text.slice(..))
                .primary();
            (scalar, aligned, final_range)
        }

        let insert = Some(Tendril::from("X"));
        let cases = [
            MappingCase {
                name: "point insertion strictly before",
                range: Range::point(1),
                change: (0, 0, insert.clone()),
                scalar: Range::point(2),
                aligned: Range::point(2),
                final_range: Range::point(2),
            },
            MappingCase {
                name: "point insertion strictly after",
                range: Range::point(1),
                change: (2, 2, insert.clone()),
                scalar: Range::point(1),
                aligned: Range::point(1),
                final_range: Range::point(1),
            },
            MappingCase {
                name: "point insertion",
                range: Range::point(1),
                change: (1, 1, insert.clone()),
                scalar: Range::point(2),
                aligned: Range::point(2),
                final_range: Range::point(2),
            },
            MappingCase {
                name: "forward lower-boundary insertion",
                range: Range::new(1, 3),
                change: (1, 1, insert.clone()),
                scalar: Range::new(2, 4),
                aligned: Range::new(2, 4),
                final_range: Range::new(2, 4),
            },
            MappingCase {
                name: "forward upper-boundary insertion",
                range: Range::new(1, 3),
                change: (3, 3, insert.clone()),
                scalar: Range::new(1, 3),
                aligned: Range::new(1, 3),
                final_range: Range::new(1, 3),
            },
            MappingCase {
                name: "backward interior insertion",
                range: Range::new(3, 1),
                change: (2, 2, insert.clone()),
                scalar: Range::new(4, 1),
                aligned: Range::new(4, 1),
                final_range: Range::new(4, 1),
            },
            MappingCase {
                name: "forward interior insertion",
                range: Range::new(1, 3),
                change: (2, 2, insert.clone()),
                scalar: Range::new(1, 4),
                aligned: Range::new(1, 4),
                final_range: Range::new(1, 4),
            },
            MappingCase {
                name: "backward lower-boundary insertion",
                range: Range::new(3, 1),
                change: (1, 1, insert.clone()),
                scalar: Range::new(4, 2),
                aligned: Range::new(4, 2),
                final_range: Range::new(4, 2),
            },
            MappingCase {
                name: "backward upper-boundary insertion",
                range: Range::new(3, 1),
                change: (3, 3, insert.clone()),
                scalar: Range::new(3, 1),
                aligned: Range::new(3, 1),
                final_range: Range::new(3, 1),
            },
            MappingCase {
                name: "point deletion",
                range: Range::point(2),
                change: (1, 3, None),
                scalar: Range::point(1),
                aligned: Range::point(1),
                final_range: Range::point(1),
            },
            MappingCase {
                name: "point deletion at lower boundary",
                range: Range::point(1),
                change: (1, 3, None),
                scalar: Range::point(1),
                aligned: Range::point(1),
                final_range: Range::point(1),
            },
            MappingCase {
                name: "point deletion at upper boundary",
                range: Range::point(3),
                change: (1, 3, None),
                scalar: Range::point(1),
                aligned: Range::point(1),
                final_range: Range::point(1),
            },
            MappingCase {
                name: "forward interior deletion",
                range: Range::new(0, 3),
                change: (1, 2, None),
                scalar: Range::new(0, 2),
                aligned: Range::new(0, 2),
                final_range: Range::new(0, 2),
            },
            MappingCase {
                name: "backward interior deletion",
                range: Range::new(3, 0),
                change: (1, 2, None),
                scalar: Range::new(2, 0),
                aligned: Range::new(2, 0),
                final_range: Range::new(2, 0),
            },
            MappingCase {
                name: "forward lower-boundary deletion",
                range: Range::new(1, 3),
                change: (1, 2, None),
                scalar: Range::new(1, 2),
                aligned: Range::new(1, 2),
                final_range: Range::new(1, 2),
            },
            MappingCase {
                name: "backward lower-boundary deletion",
                range: Range::new(3, 1),
                change: (1, 2, None),
                scalar: Range::new(2, 1),
                aligned: Range::new(2, 1),
                final_range: Range::new(2, 1),
            },
            MappingCase {
                name: "forward upper-boundary deletion",
                range: Range::new(1, 3),
                change: (2, 3, None),
                scalar: Range::new(1, 2),
                aligned: Range::new(1, 2),
                final_range: Range::new(1, 2),
            },
            MappingCase {
                name: "backward upper-boundary deletion",
                range: Range::new(3, 1),
                change: (2, 3, None),
                scalar: Range::new(2, 1),
                aligned: Range::new(2, 1),
                final_range: Range::new(2, 1),
            },
            MappingCase {
                name: "point unequal replacement at lower boundary",
                range: Range::point(1),
                change: (1, 3, Some(Tendril::from("X"))),
                scalar: Range::point(1),
                aligned: Range::point(1),
                final_range: Range::point(1),
            },
            MappingCase {
                name: "point unequal replacement at upper boundary",
                range: Range::point(3),
                change: (1, 3, Some(Tendril::from("X"))),
                scalar: Range::point(2),
                aligned: Range::point(2),
                final_range: Range::point(2),
            },
            MappingCase {
                name: "point unequal replacement in interior",
                range: Range::point(2),
                change: (1, 3, Some(Tendril::from("X"))),
                scalar: Range::point(2),
                aligned: Range::point(2),
                final_range: Range::point(2),
            },
            MappingCase {
                name: "forward lower-boundary unequal replacement",
                range: Range::new(1, 3),
                change: (1, 2, Some(Tendril::from("XY"))),
                scalar: Range::new(1, 4),
                aligned: Range::new(1, 4),
                final_range: Range::new(1, 4),
            },
            MappingCase {
                name: "backward lower-boundary unequal replacement",
                range: Range::new(3, 1),
                change: (1, 2, Some(Tendril::from("XY"))),
                scalar: Range::new(4, 1),
                aligned: Range::new(4, 1),
                final_range: Range::new(4, 1),
            },
            MappingCase {
                name: "forward upper-boundary unequal replacement",
                range: Range::new(1, 3),
                change: (2, 3, Some(Tendril::from("XY"))),
                scalar: Range::new(1, 4),
                aligned: Range::new(1, 4),
                final_range: Range::new(1, 4),
            },
            MappingCase {
                name: "backward upper-boundary unequal replacement",
                range: Range::new(3, 1),
                change: (2, 3, Some(Tendril::from("XY"))),
                scalar: Range::new(4, 1),
                aligned: Range::new(4, 1),
                final_range: Range::new(4, 1),
            },
            MappingCase {
                name: "forward equal replacement",
                range: Range::new(1, 3),
                change: (1, 3, Some(Tendril::from("YZ"))),
                scalar: Range::new(1, 3),
                aligned: Range::new(1, 3),
                final_range: Range::new(1, 3),
            },
            MappingCase {
                name: "backward equal whole-range replacement",
                range: Range::new(3, 1),
                change: (1, 3, Some(Tendril::from("YZ"))),
                scalar: Range::new(3, 1),
                aligned: Range::new(3, 1),
                final_range: Range::new(3, 1),
            },
            MappingCase {
                name: "point before equal replacement",
                range: Range::point(0),
                change: (1, 3, Some(Tendril::from("YZ"))),
                scalar: Range::point(0),
                aligned: Range::point(0),
                final_range: Range::point(0),
            },
            MappingCase {
                name: "point inside equal replacement",
                range: Range::point(2),
                change: (1, 3, Some(Tendril::from("YZ"))),
                scalar: Range::point(2),
                aligned: Range::point(2),
                final_range: Range::point(2),
            },
            MappingCase {
                name: "point after equal replacement",
                range: Range::point(4),
                change: (1, 3, Some(Tendril::from("YZ"))),
                scalar: Range::point(4),
                aligned: Range::point(4),
                final_range: Range::point(4),
            },
            MappingCase {
                name: "backward unequal replacement",
                range: Range::new(3, 1),
                change: (1, 3, Some(Tendril::from("Z"))),
                scalar: Range::new(2, 1),
                aligned: Range::new(2, 1),
                final_range: Range::new(2, 1),
            },
            MappingCase {
                name: "backward whole-range deletion",
                range: Range::new(3, 1),
                change: (1, 3, None),
                scalar: Range::point(1),
                aligned: Range::point(1),
                final_range: Range::point(1),
            },
            MappingCase {
                name: "forward whole-range deletion",
                range: Range::new(1, 3),
                change: (1, 3, None),
                scalar: Range::point(1),
                aligned: Range::point(1),
                final_range: Range::point(1),
            },
        ];
        for case in cases {
            let (scalar, aligned, final_range) = phases("abcd", case.range, case.change);
            assert_eq!(scalar, case.scalar, "{} scalar", case.name);
            assert_eq!(aligned, case.aligned, "{} aligned", case.name);
            assert_eq!(final_range, case.final_range, "{} final", case.name);
        }
    }

    #[test]
    fn unequal_replacement_maps_interior_endpoints_by_bound_role() {
        fn assert_phases(range: Range, replacement: &str, expected: Range) {
            let old_text = Rope::from("abcdef");
            let transaction = Transaction::change(
                &old_text,
                std::iter::once((1, 5, Some(Tendril::from(replacement)))),
            );
            let scalar_selection = Selection::from(range).map_no_normalize(transaction.changes());
            assert_eq!(scalar_selection.primary(), expected);

            let mut new_text = old_text.clone();
            assert!(transaction.changes().apply(&mut new_text));
            assert_eq!(
                scalar_selection
                    .primary()
                    .grapheme_aligned(new_text.slice(..)),
                expected
            );
            assert_eq!(
                Selection::from(range)
                    .map(transaction.changes(), new_text.slice(..))
                    .primary(),
                expected
            );
        }

        for (range, expected) in [
            (Range::new(2, 6), Range::new(1, 3)),
            (Range::new(6, 2), Range::new(3, 1)),
            (Range::new(0, 4), Range::new(0, 2)),
            (Range::new(4, 0), Range::new(2, 0)),
            (Range::new(2, 4), Range::new(1, 2)),
            (Range::new(4, 2), Range::new(2, 1)),
        ] {
            assert_phases(range, "X", expected);
        }

        for range in [Range::new(2, 4), Range::new(4, 2)] {
            assert_phases(range, "WXYZ", range);
        }
    }

    #[test]
    fn mapping_realigns_after_grapheme_segmentation_changes() {
        struct SegmentationCase {
            name: &'static str,
            source: &'static str,
            range: Range,
            inserted: &'static str,
            scalar: Range,
            aligned: Range,
        }

        let cases = [
            SegmentationCase {
                name: "combining mark joins left at upper edge",
                source: "ab",
                range: Range::new(0, 1),
                inserted: "\u{301}",
                scalar: Range::new(0, 1),
                aligned: Range::new(0, 2),
            },
            SegmentationCase {
                name: "variation selector joins left at upper edge",
                source: "❤x",
                range: Range::new(0, 1),
                inserted: "\u{fe0f}",
                scalar: Range::new(0, 1),
                aligned: Range::new(0, 2),
            },
            SegmentationCase {
                name: "ZWJ joins across lower edge",
                source: "👩💻",
                range: Range::new(1, 2),
                inserted: "\u{200d}",
                scalar: Range::new(2, 3),
                aligned: Range::new(0, 3),
            },
            SegmentationCase {
                name: "ZWJ joins across upper edge",
                source: "👩💻",
                range: Range::new(0, 1),
                inserted: "\u{200d}",
                scalar: Range::new(0, 1),
                aligned: Range::new(0, 3),
            },
            SegmentationCase {
                name: "combining mark joins left at lower edge",
                source: "ab",
                range: Range::new(1, 2),
                inserted: "\u{301}",
                scalar: Range::new(2, 3),
                aligned: Range::new(2, 3),
            },
            SegmentationCase {
                name: "variation selector joins left at lower edge",
                source: "❤x",
                range: Range::new(1, 2),
                inserted: "\u{fe0f}",
                scalar: Range::new(2, 3),
                aligned: Range::new(2, 3),
            },
            SegmentationCase {
                name: "combining sequence does not join at upper edge",
                source: "ab",
                range: Range::new(0, 1),
                inserted: "X\u{301}",
                scalar: Range::new(0, 1),
                aligned: Range::new(0, 1),
            },
            SegmentationCase {
                name: "combining sequence does not join at lower edge",
                source: "ab",
                range: Range::new(1, 2),
                inserted: "X\u{301}",
                scalar: Range::new(3, 4),
                aligned: Range::new(3, 4),
            },
            SegmentationCase {
                name: "variation-selector sequence does not join at upper edge",
                source: "ab",
                range: Range::new(0, 1),
                inserted: "X\u{fe0f}",
                scalar: Range::new(0, 1),
                aligned: Range::new(0, 1),
            },
            SegmentationCase {
                name: "variation-selector sequence does not join at lower edge",
                source: "ab",
                range: Range::new(1, 2),
                inserted: "X\u{fe0f}",
                scalar: Range::new(3, 4),
                aligned: Range::new(3, 4),
            },
            SegmentationCase {
                name: "ZWJ sequence does not join at upper edge",
                source: "ab",
                range: Range::new(0, 1),
                inserted: "X\u{200d}",
                scalar: Range::new(0, 1),
                aligned: Range::new(0, 1),
            },
            SegmentationCase {
                name: "ZWJ sequence does not join at lower edge",
                source: "ab",
                range: Range::new(1, 2),
                inserted: "X\u{200d}",
                scalar: Range::new(3, 4),
                aligned: Range::new(3, 4),
            },
            SegmentationCase {
                name: "upper-edge insertion does not join",
                source: "ab",
                range: Range::new(0, 1),
                inserted: "X",
                scalar: Range::new(0, 1),
                aligned: Range::new(0, 1),
            },
            SegmentationCase {
                name: "lower-edge insertion does not join",
                source: "ab",
                range: Range::new(1, 2),
                inserted: "X",
                scalar: Range::new(2, 3),
                aligned: Range::new(2, 3),
            },
        ];
        for case in cases {
            let old_text = Rope::from(case.source);
            let transaction = Transaction::change(
                &old_text,
                std::iter::once((1, 1, Some(Tendril::from(case.inserted)))),
            );
            let scalar = Selection::from(case.range)
                .map_no_normalize(transaction.changes())
                .primary();
            let mut new_text = old_text.clone();
            assert!(transaction.changes().apply(&mut new_text));
            let aligned = scalar.grapheme_aligned(new_text.slice(..));
            assert_eq!(scalar, case.scalar, "{} scalar", case.name);
            assert_eq!(aligned, case.aligned, "{} aligned", case.name);
            assert_eq!(
                Selection::from(case.range)
                    .map(transaction.changes(), new_text.slice(..))
                    .primary(),
                case.aligned,
                "{} final",
                case.name
            );
        }
    }

    #[test]
    fn mapping_collisions_preserve_primary_and_collapse_forward() {
        let old_text = Rope::from("abcdef");
        let transaction = Transaction::change(&old_text, std::iter::once((1, 4, None)));
        let selection = Selection::new(
            smallvec![Range::new(1, 2), Range::new(4, 3), Range::point(6)],
            1,
        );
        let mut new_text = old_text.clone();
        assert!(transaction.changes().apply(&mut new_text));

        let scalar = selection.clone().map_no_normalize(transaction.changes());
        assert_eq!(
            scalar.ranges(),
            &[Range::point(1), Range::point(1), Range::point(3)]
        );
        let mapped = selection.map(transaction.changes(), new_text.slice(..));
        assert_eq!(mapped.ranges(), &[Range::point(1), Range::point(3)]);
        assert_eq!(mapped.primary_index(), 0);
        assert_eq!(mapped.primary().direction(), Direction::Forward);
    }

    #[test]
    fn mapping_collision_primary_is_permutation_independent() {
        let old_text = Rope::from("abcdef");
        let transaction = Transaction::change(&old_text, std::iter::once((1, 4, None)));
        let primary = Range::point(2);
        let ranges = [Range::point(1), primary, Range::point(4)];
        let mut new_text = old_text.clone();
        assert!(transaction.changes().apply(&mut new_text));

        for permutation in [
            [ranges[0], ranges[1], ranges[2]],
            [ranges[0], ranges[2], ranges[1]],
            [ranges[1], ranges[0], ranges[2]],
            [ranges[1], ranges[2], ranges[0]],
            [ranges[2], ranges[0], ranges[1]],
            [ranges[2], ranges[1], ranges[0]],
        ] {
            let primary_index = permutation
                .iter()
                .position(|&range| range == primary)
                .unwrap();
            let mapped = Selection::new_unaligned(permutation.into_iter().collect(), primary_index)
                .map(transaction.changes(), new_text.slice(..));
            assert_eq!(mapped.ranges(), &[Range::point(1)]);
            assert_eq!(mapped.primary_index(), 0);
        }
    }

    #[test]
    fn mapping_secondary_collision_preserves_separate_primary() {
        let old_text = Rope::from("abcdef");
        let transaction = Transaction::change(&old_text, std::iter::once((1, 4, None)));
        let primary = Range::point(6);
        let secondary = [Range::point(1), Range::point(2), Range::point(4)];
        let mut new_text = old_text.clone();
        assert!(transaction.changes().apply(&mut new_text));

        for permutation in [
            [secondary[0], secondary[1], secondary[2]],
            [secondary[0], secondary[2], secondary[1]],
            [secondary[1], secondary[0], secondary[2]],
            [secondary[1], secondary[2], secondary[0]],
            [secondary[2], secondary[0], secondary[1]],
            [secondary[2], secondary[1], secondary[0]],
        ] {
            let mut ranges: SmallVec<[Range; 1]> = permutation.into_iter().collect();
            ranges.push(primary);
            let mapped =
                Selection::new_unaligned(ranges, 3).map(transaction.changes(), new_text.slice(..));
            assert_eq!(mapped.ranges(), &[Range::point(1), Range::point(3)]);
            assert_eq!(mapped.primary_index(), 1);
        }
    }

    #[test]
    fn replacement_realigns_special_grapheme_sequences() {
        for (name, source, replacement, expected_aligned) in [
            (
                "combining replacement joins left",
                "ab",
                "\u{301}",
                Range::new(0, 2),
            ),
            (
                "ZWJ replacement joins left and right",
                "👩x",
                "\u{200d}💻",
                Range::new(0, 3),
            ),
            (
                "variation-selector replacement joins left",
                "❤x",
                "\u{fe0f}x",
                Range::new(0, 2),
            ),
        ] {
            let old_text = Rope::from(source);
            let transaction = Transaction::change(
                &old_text,
                std::iter::once((1, 2, Some(Tendril::from(replacement)))),
            );
            let scalar = Selection::from(Range::new(0, 1))
                .map_no_normalize(transaction.changes())
                .primary();
            assert_eq!(scalar, Range::new(0, 1), "{name} scalar");
            let mut new_text = old_text.clone();
            assert!(transaction.changes().apply(&mut new_text));
            assert_eq!(
                scalar.grapheme_aligned(new_text.slice(..)),
                expected_aligned,
                "{name} aligned"
            );
            assert_eq!(
                Selection::from(Range::new(0, 1))
                    .map(transaction.changes(), new_text.slice(..))
                    .primary(),
                expected_aligned,
                "{name} final"
            );
        }
    }

    #[test]
    fn affinity_helpers_are_total_and_right_first() {
        let text = Rope::from("a🦀");
        assert_eq!(char_at_edge(text.slice(..), 0), Ok(Some('a')));
        assert_eq!(char_at_edge(text.slice(..), 2), Ok(None));
        assert!(char_at_edge(text.slice(..), 3).is_err());
        assert_eq!(byte_range_at_edge(text.slice(..), 0), Ok(0..1));
        assert_eq!(byte_range_at_edge(text.slice(..), 1), Ok(1..5));
        assert_eq!(byte_range_at_edge(text.slice(..), 2), Ok(5..5));
        assert_eq!(
            adjacent_char_positions(text.slice(..), 1)
                .unwrap()
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert_eq!(
            adjacent_char_positions(text.slice(..), 2)
                .unwrap()
                .collect::<Vec<_>>(),
            vec![1]
        );
    }
}
