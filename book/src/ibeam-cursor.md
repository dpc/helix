# I-beam cursor and edge-selection model

Status: normative. Product decisions D1 through D10 are final.

This document defines Helix's document-cursor and selection behavior. The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are normative.


## 1. Scope

This model applies to document views in Normal, Select, and Insert modes, to every primary and secondary selection, and to keyboard, mouse, command, LSP, DAP, tree-sitter, and transaction paths that create or consume document positions.

Prompt and picker input fields also use an I-beam by default, but their private string selection model is outside this specification.


## 2. Terms and representation

- A **character index** is a Rope `char` offset in `[0, text.len_chars()]`.
- An **edge** is a position between Unicode scalar values. Edge `0` is before the first scalar; edge `text.len_chars()` is EOF.
- A **grapheme edge** is an edge at an extended grapheme-cluster boundary. CRLF is one grapheme for cursor movement.
- A **range** is `(anchor, head)`. Both fields are edges.
- The **cursor** or **beam** is at `head` exactly.
- A range selects the half-open interval `[min(anchor, head), max(anchor, head))`.
- A **point** is a range whose anchor equals its head. It selects no text.
- A range is **forward** when `anchor` is at or before `head`; it is **backward** otherwise. A point is forward by convention.
- A **selection set** is one or more sorted, normalized ranges and identifies one range as primary.

Conceptually:

```text
text:      a b c
edges:   0 1 2 3

(0, 0)  |abc       point at BOF
(0, 2)  [ab|]c     forward selection, beam at edge 2
(2, 0)  |[ab]c     backward selection, beam at edge 0
(3, 3)  abc|       point at EOF
```

Indices remain scalar-value offsets because Rope and protocol conversions use them internally. User-visible movement and selection boundaries MUST be grapheme aligned.


## 3. Representation invariants

Every selection stored on a document MUST satisfy all of these invariants:

1. It contains at least one range.
2. Every anchor and head is a valid character index.
3. Every anchor and head is a grapheme edge.
4. Point ranges are valid at every grapheme edge, including BOF, EOL, empty lines, and EOF.
5. Nonempty ranges remain nonempty when aligned to graphemes.
6. Ranges are sorted by their lower edge.
7. Ranges do not overlap under the selection-set overlap rule.
8. The primary index names the logical primary range after sorting, alignment, mapping, and merging.
9. A range preserves direction while it remains nonempty. If mapping, alignment, or a command collapses it to a point, the point is forward by convention. An explicit flip and D10 collision normalization are the other direction-changing cases.

Construction and validation follow these rules:

- A valid scalar range whose endpoint falls inside a grapheme is normalized outward: the lower bound snaps to the preceding grapheme edge and the upper bound snaps to the following grapheme edge. Direction is then restored. This may select more scalar values than an incoming regex, LSP, or tree-sitter range, but never splits a displayed grapheme.
- A point inside a grapheme snaps to its preceding edge and remains a point.
- An endpoint outside `[0, text.len_chars()]` is invalid. External conversions return failure and leave the current selection unchanged. Internal commands MUST propagate an error or no-op before calling `Document::set_selection`; infallible core constructors MAY assert that validated callers uphold the bound. No path silently clamps an invalid stored endpoint.
- Screen-coordinate conversion is the only clamping path: coordinates beyond content map to the nearest valid line edge or EOF according to the coordinate API.
- Outgoing protocol conversion preserves the exact text selected by the normalized stored range. It does not promise to reproduce an unaligned external source range that could not be represented without splitting a grapheme.

Two identical points normalize to one cursor. Adjacent nonempty half-open ranges do not overlap. A point inside a nonempty range or at its lower edge overlaps it; a point at its upper edge does not. These results are independent of input order.

When ranges merge, their union uses the D10 direction rule. If any merged range was primary, the union is primary. A merge group without the primary cannot change which separate range remains primary. Normalization MUST produce the same bounds, direction, and primary identity for every permutation of equivalent input ranges.

A new nonempty document starts with `Range::point(0)`, not a synthetic selection of its first grapheme. An empty document also starts at `Range::point(0)`.

### Mapping through edits

Mapping has exactly three ordered phases:

1. Map every range's scalar anchor and head independently through the change set without merging ranges. Endpoint association treats ordered lower and upper edges by their role, not by whether they are stored in `anchor` or `head`.
2. Align every mapped range against the **new** text. A point snaps to the preceding grapheme edge and stays empty; a nonempty range expands outward while preserving its pre-collision direction. Grapheme indivisibility overrides scalar endpoint stickiness.
3. Run one D10 collision normalization over the complete aligned set. No mapping or alignment helper may normalize earlier.

This ordering is normative. `Selection::map` MUST behave like `map_no_normalize`, followed by new-text grapheme alignment, followed by exactly one normalization.

- An insertion strictly before an edge shifts it by the insertion length. An insertion strictly after it does not move it.
- An insertion exactly at a point moves the point after the inserted text.
- For a nonempty range, insertion at either boundary remains outside the selection: the lower edge moves after an insertion at that edge, while the upper edge remains before it. Insertion strictly inside the range becomes selected.
- A deletion maps every point at or inside the closed deleted boundary interval to the deletion start. Positions after the deletion shift left.
- For replacement `[x, y)` with inserted length `n`, a point at `x` remains at `x`, a point at `y` maps to `x + n`, and a point strictly inside maps to the same relative offset when old and new lengths match; otherwise it maps to `x + n`.
- A nonempty range boundary strictly inside an equal-length replacement keeps its relative offset. For an unequal replacement, an affected lower boundary maps to `x` and an affected upper boundary maps to `x + n`. A range exactly equal to the replaced interval therefore selects exactly the replacement text and keeps its original direction.
- Scalar mapping can turn ranges into points or make ranges collide. Grapheme alignment can expand them again or create additional collisions. Only phase 3 applies D10. If the primary collides, the merged union remains primary and uses the primary's direction immediately before phase 3.
- Insertion or replacement can change grapheme segmentation across an endpoint. For example, a combining mark inserted at the upper edge of a selected base scalar joins that base and is selected after outward alignment even though upper-edge stickiness excluded it in phase 1. A ZWJ or variation selector inserted at a lower edge can join the right-hand grapheme and pull the aligned lower edge left. A point mapped after inserted text can snap left if the insertion joins a grapheme spanning that scalar edge. The editor never stores an edge inside the resulting grapheme.

Tests MUST cover pure insertion, deletion, equal and unequal replacement at both boundaries and strictly inside; forward and backward forms; points; forward and backward whole-range deletion and replacement-to-empty collapsing to an identical forward point; multiple collisions; primary and secondary collisions; and every permutation of equivalent input ranges. At both lower and upper endpoints, include edits containing combining marks, ZWJ sequences, and variation selectors, with cases that join left, join right, and do not join. Assert the phase-1 scalar result, phase-2 aligned result, and final D10 result separately.


## 4. Cursor affinity and object lookup

**D8 — global edge affinity.** The beam has **right affinity** for character-oriented queries. At edge `p`, the character or token “at the cursor” starts at `p`. Ordinary character information, word objects, hover, completion, rename, code action, syntax-layer, and comment-token queries do not fall back left at EOF. LSP point requests may send EOF as a valid position and let the server answer. Only explicitly adjacency-based commands inspect both sides: brace and surround discovery check right first and then left; implicit `goto_file` path discovery may inspect a path adjacent on either side. No fallback creates an invalid edge.


**D6 — word text objects on whitespace/EOL/EOF.** Use right affinity and return a point when the right-hand character is whitespace or EOL, and at EOF. The command does not fall back left.

Under this rule:

- character information reports the grapheme to the right;
- hover, completion, rename, code action, syntax-layer lookup, and comment-token lookup query the right-hand position and return no local object at EOF;
- an edge at a half-open object's end is not inside that object;
- line ownership at a line-ending edge follows Rope's position-to-line rule;
- brace matching is adjacency-specific rather than ordinary ownership: check the right-hand bracket first and then the immediately adjacent left-hand bracket;
- EOF has no character to its right; only the enumerated adjacency commands inspect left.

Protocol ranges remain half-open. Forward and backward Helix ranges with the same bounds MUST convert to the same LSP range. UTF-8, UTF-16, and UTF-32 position conversions MUST preserve zero-width points and multibyte boundaries. Command-line expansions expose the same model: `%{cursor_line}` and `%{cursor_column}` derive from `head`; `%{selection}` is the exact primary fragment and is empty for a point; selection start/end line variables use the primary half-open line range, with a point naming its containing line.


## 5. Selection semantics

Commands that explicitly select a target—regex search matches, diagnostics, syntax nodes, text objects, references, diff hunks, and picker locations with ranges—MUST store the target's natural half-open range. They MUST NOT add or remove one grapheme merely to emulate a block cursor.

**D9 — target cursor placement.** A directional target navigation places `head` at the arrival edge: forward navigation uses the target's upper edge and backward navigation uses its lower edge. A nondirectional location jump, including LSP definition/declaration and picker location, places `head` at the target's lower edge so the cursor shows its beginning. An explicit select-object command creates a forward target range unless it is extending an existing selection. Repeating a directional command continues from its arrival edge.

Extending a selection moves only `head`; `anchor` remains fixed. Crossing the anchor reverses direction naturally and may produce a point at the crossing edge. Flipping swaps anchor and head without changing selected text. Ensuring forward direction changes only direction.

**D10 — merged direction and primary.** If an overlap group contains the primary range, the union keeps the primary's pre-merge direction. Otherwise the winner is the range with the smallest lower edge; ties choose the widest range, then forward direction. A forward union has `anchor = union.from` and `head = union.to`; a backward union has the opposite assignment. This tie-break is independent of insertion order.

Collapsing a selection produces `Range::point(head)` unless a command explicitly names the lower or upper boundary.

**D1 — Normal-mode motion result classes.**

1. A **linear selecting motion** replaces each input range with the half-open interval from its old head to its destination. Horizontal grapheme, word/subword, find/till, line-bound, paragraph, and file-bound motions belong here. It selects the text crossed by the beam.
2. A **relocation motion** replaces each input range with a point at its destination. Physical/visual vertical movement, requested column, numbered line, window-row, jumplist, and history-location navigation belong here.
3. A **target-selection motion** selects the target's natural range. Search matches, diagnostics, changes, syntax objects, LSP targets, and jump-label targets belong here.

Select-mode variants extend from the existing anchor to the same destination instead of applying the Normal-mode result shape. Insert-mode cursor movement relocates a point and MUST NOT create a selection.

Counts repeat destination calculation. A zero count, if accepted by an internal API, returns the original range unchanged.


## 6. Mode transitions

The cursor remains an edge in every mode. A mode transition MUST NOT translate the cursor by one grapheme merely to imitate a block cursor.

**D2 — transition collapse policy.**

- Normal to Select preserves every current anchor and head. Subsequent Select-mode motions extend from the preserved anchor.
- Select to Normal preserves ranges; it changes only the interpretation of motion commands.
- Normal or Select to Insert collapses each range to its head before the first insertion.
- Normal or Select to Append also collapses each range to its head. `insert_mode` and `append_mode` therefore have the same insertion edge.
- Insert to Normal preserves each insertion point. It does not move left and does not run legacy `restore_cursor` surgery.



## 7. Insertion, deletion, and replacement

Text insertion occurs exactly at each head. Normal insertion maps a point after inserted text. For a nonempty half-open range, inserting at its lower edge keeps the inserted text outside the range and inserting at its upper edge keeps it outside the range, consistent with endpoint stickiness.

Auto-pair insertion, skip-over, newline expansion, and paired backspace MUST return a point when the input is a point. Paired backspace in `(|)` deletes both pair characters and leaves `|`; it MUST NOT select neighboring text.

Insert-mode backspace deletes the grapheme immediately left of each beam. Insert-mode forward delete deletes the grapheme immediately right. At BOF or EOF respectively, these are no-ops. Word and line kills use half-open intervals ending or starting at the beam and never manufacture an EOF character.

For a nonempty range, delete/change/replace operate on exactly `[from, to)`. Changes map all resulting beams to valid grapheme edges and coalesce collisions deterministically.

**D3 — operator behavior at a point.** A character-oriented selection operator materializes the grapheme immediately right of a point as its effective operand. This applies consistently to delete, change, yank, replace, case conversion, shell pipe, increment/decrement, and selection-content transforms. At EOF it is a no-op. Implement effective-operand resolution centrally rather than only in delete/change. Line-oriented commands retain their containing-line rule.

Line-oriented commands—indent, unindent, line comments, join, and line formatting—operate on the line containing a point. This is an explicit exception because their operand is a line set, not selected text. Range-formatting from a point expands to the complete containing line, including its line ending when present.

Surround-add on a point inserts an empty pair around the beam. Surround-replace/delete require an actual or discoverable surround according to their command contract.


## 8. Paste semantics

**D4 — paste boundaries.**

- Following the patch's explicit static-command conversion, `paste_before` and `paste_after` both insert at `head`. They are aliases under the edge model and their descriptions MUST say so.
- Explicit cursor paste, bracketed paste in Insert/Select, and middle-click paste insert at `head`.
- A linewise cursor paste inserts at the start of the line containing `head`; both aliases use that same boundary. It never inserts into the middle of a line and never computes `head + 1`.
- The inserted text becomes the resulting Normal-mode selection using its exact half-open bounds. Its direction matches the input range: forward input has `head` at the inserted text's upper edge; backward input has `head` at its lower edge. Insert-mode paste leaves a point after the inserted text.

Static, typable, clipboard, primary-clipboard, and register variants MUST share this implementation and naming.


## 9. Lines, line endings, empty lines, and EOF

EOL is the edge immediately before a line-ending grapheme. “Line end” moves to EOL. “Line end including newline” moves to the edge immediately after the line-ending grapheme when one exists; on an unterminated final line it moves to EOF. It MUST NOT produce `text.len_chars() + 1`.

Selecting a complete terminated line includes its line ending. Selecting an unterminated final line ends at EOF. An empty line can be represented by a point at its content boundary or by a range covering its line ending, depending on whether the command selects the cursor position or the whole line.

EOF is an ordinary valid point. Commands that need a right-hand grapheme no-op at EOF unless their documented affinity supplies a left fallback. Tree-sitter nodes and text objects whose exclusive end equals EOF are valid.

CRLF is traversed and deleted as one grapheme. Rendering may display a single configured newline marker for it.


## 10. Graphemes and multibyte text

Character-wise cursor movement advances by extended grapheme clusters, not Unicode scalar values, bytes, or display cells. Combining sequences, emoji ZWJ sequences, variation selectors, regional-indicator sequences, wide glyphs, tabs, and CRLF MUST remain indivisible under movement, deletion, range alignment, and cursor rendering.

Rope storage uses scalar offsets; terminal layout uses display cells; LSP conversion uses the negotiated encoding. Conversions MUST round-trip boundaries without changing the selected half-open text.


## 11. Multiple selections

Every cursor-affecting command applies independently to all ranges unless its command contract explicitly keeps only the primary range. The result is then normalized.

- Direction is preserved per range through command transformation, scalar mapping, and grapheme alignment while the range remains nonempty. A collapsed point becomes forward by convention; an explicit flip and D10 merge are the other exceptions.
- Secondary cursors are points or heads under the same rules as the primary.
- Insertions and deletions are computed against the original document and applied as one transaction.
- Ranges that collide after movement or edits merge deterministically.
- If the primary participates in a merge, the merged result remains primary.
- Copy-selection-to-adjacent-line copies exact edge coordinates and exact width; it does not subtract a block-cursor cell.
- Auto-pair and grapheme operations return valid points for every cursor.


## 12. Mouse behavior

A plain left click places `Range::point(clicked_edge)` and makes it primary. Alt-click adds a point cursor. A Select-mode click extends the primary range to the clicked edge according to the transition policy. Dragging creates a half-open range from the press edge to the current edge and preserves drag direction.

A click has an empty range; any nonempty drag, including a one-grapheme drag, is eligible for mouse-yank. Mouse-yank MUST test `!range.is_empty()`, not a block-era minimum length.

Right-click and middle-click placement use exact clicked edges. Middle-click paste inserts at that edge. Screen-coordinate conversion MUST clamp to valid grapheme edges at EOL and EOF and MUST ignore virtual text when the gesture requests a document position. A click in any terminal cell occupied by a tab or wide grapheme maps to that grapheme's left edge; only a cell at or after its visual end maps to the following edge.


## 13. Rendering

The primary document cursor is rendered at `head`. A focused terminal renders the primary Bar or Underline cursor through the terminal backend. Secondary cursors and an unfocused primary require an explicit in-buffer marker because terminals expose one hardware cursor.

Selection highlighting covers exactly `[from, to)` and never includes a cursor cell merely to emulate a block cursor. A block-shaped configuration, if allowed, changes only cursor appearance; it does not change edge representation or selection bounds.

At EOF, an unfocused or secondary point cursor needs a virtual one-cell marker without creating an out-of-range document span. Rendering code MUST distinguish virtual EOF cells from Rope character ranges.


**D7 — selected-newline visualization.** Selection changes style only. If newline whitespace rendering is enabled, the renderer continues to use the configured newline glyph; selection does not introduce a separate hardcoded glyph.

Selected line endings MAY display the configured newline glyph when newline whitespace rendering is enabled. Rendering MUST identify selection membership directly; it MUST NOT infer “selected” from the presence of an arbitrary overlay style. Diagnostics, links, tabstops, document highlights, and brace overlays do not make a newline selected.

Brace matching checks bracket adjacency as defined in section 4. Cursor, selection, diagnostic, and brace overlays MUST compose without overlapping-span violations or accidental style-dependent behavior.


## 14. Selection-change consumers

`Document::set_selection` normalizes once and emits `SelectionDidChange` with the stored edge selection. Subscribers MUST consume `head` and half-open bounds without widening a point. Document highlights, breadcrumbs/symbols, code-action hints, signature help, snippets, completion, statusline, gutters, and diagnostics MUST tolerate a point at every valid edge, including EOF. A subscriber MUST NOT cause a second semantic selection change merely to emulate block width.

Commands that choose between an explicit selection and cursor-adjacent discovery MUST test `is_empty()`, never a scalar or grapheme length threshold. `goto_file` auto-detects a path only for a point; any nonempty selection, including one grapheme, is the exact path operand. Rename uses any nonempty primary fragment verbatim and invokes the D6 word object only from a point.


## 15. Configuration and compatibility

**D5 — shape overrides.** Edge semantics are unconditional, while cursor shape remains cosmetic and configurable. The defaults for Normal, Select, Insert, prompt, and picker cursors are `bar`. Users MAY configure `block` or `underline`; those values MUST NOT reactivate block-selection arithmetic.

A partially specified `[editor.cursor-shape]` fills missing modes with the new `bar` default. This is a behavior migration and MUST appear in release notes. Serialization/deserialization tests MUST cover empty, partial, and complete maps.


Themes control cursor and selection styles but cannot change cursor geometry or range semantics. The selected-newline glyph uses the existing whitespace configuration unless a separately documented setting is introduced.


## 16. Command-family contract

The following contract is normative:

| Family | Result/operand |
| --- | --- |
| Character, word, subword, find/till, line/paragraph/file motions | Linear selecting or extending motion per section 5 |
| Vertical, column, viewport, numbered-line, jumplist/history moves | Relocation or extension per section 5 |
| Search, diagnostic, diff, syntax, LSP, text-object targets | Natural half-open target range |
| Collapse, flip, ensure-forward, merge, split, trim | Pure range transforms; points remain valid |
| Delete/change/yank/replace/case/pipe/increment | Exact nonempty ranges; point behavior per section 7 |
| Insert, append, open line, typed insert/delete | Exact head edges; resulting points |
| Paste/register/clipboard/bracketed/middle-click | Boundaries in section 8 |
| Indent, comments, join, line formatting | Lines overlapped by ranges; containing line for a point |
| Surround | Exact half-open range; empty add inserts a pair |
| LSP/DAP/tree-sitter/character info/brace match | Affinity in section 4; no block widening |
| Undo/redo/history, macros, registers, view/buffer commands | Preserve and replay edge ranges without reinterpretation |


## 17. Rationale

The previous block-cursor implementation stored half-open ranges but interpreted `head` indirectly as a character cell. It widened points, shifted anchors when direction crossed, and added special EOF behavior. The edge model instead makes the stored head equal the visible cursor position. This is better because insertion, external half-open ranges, GUI-style pointing, EOF, and forward/backward selections use one representation.

Keeping appearance separate from semantics lets terminals render a block for accessibility or preference without changing editing behavior. Requiring explicit point-operand and motion-result policies prevents the most damaging failure mode: the same visible beam causing unrelated commands to operate on different hidden graphemes.


## 18. Examples

These examples are illustrative; normative rules appear above.

```text
Start:          ab|cd
extend right:   ab[c|]d
extend left:    ab|cd
flip [bc]:      a|[bc]d  ↔  a[bc|]d
EOF:            abcd|
```

With right affinity, `character-info` at `ab|cd` reports `c`; at `abcd|` it reports no character.

With the cursor-paste rule:

```text
selection:      a[bc|]d
paste_before X: a[bc|]Xd
paste_after X:  a[bc|]Xd
point:          ab|cd       # before and after both insert at this edge
```
