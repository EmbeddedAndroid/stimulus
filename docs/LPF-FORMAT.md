# LogicPort project (`.LPF`) format

This document records corpus-confirmed structure from the 17 vendor examples in
`fixtures/vendor/examples/`. It distinguishes facts needed by the importer from semantic
field mappings that remain to be recovered.

## Container grammar

- Text is Windows-1252. Fields are separated by byte `0x11`.
- A record starts with its key. Its values continue across separators until the final value,
  which is followed by `CR LF` and the next key in the same field.
- `NotesString/` is exceptional: its free-form value can contain arbitrary line breaks and
  ends at the field `/`.
- The corpus contains 79 distinct keys. `lp-lpf::keys::KNOWN_KEYS` is the executable inventory;
  importing all examples produces zero unknown-key reports.
- Repeated record keys represent lists, notably `Group`, `Row`, and `Column`.

`Group` normalizes to 36 fields, `Row` to 9, and `Column` to 4. Some versions omit trailing
empty fields: populated groups can contain 35 serialized fields, empty group slots contain
only nine empty fields, and columns contain three or four. The parser pads these trailing
omissions without discarding the original records.

## SampleData

`SampleData` has four unsigned integer metadata fields followed by a braced CSV block. The
first two metadata values are confirmed as channel count (`34`) and stored-run count; the
semantic names of metadata fields 2 and 3 remain subject to the reference/trigger mapping.

The CSV header is exactly:

```text
D0,D1,...,D31,CLK1,CLK2,Count
```

Every subsequent row has 35 fields. Channel values are `0`, `1`, or `U` (not acquired), and
`Count` is a positive unsigned RLE length. All 17 examples satisfy:

- 34 channels;
- metadata run count equals the number of CSV rows;
- every row has exactly 35 fields;
- the checked sum of all Count fields is at least the stored-run count.

The largest current example expands to 780,311,573 samples without materializing the expanded
array. `lp-lpf` retains runs directly.

## Semantic conversion status

The importer currently maps the capture, signal names, notes, cursors, acquisition and sample
settings, control selections, export/print settings, measurements, groups, interpreters, rows,
and columns into the native LPJ model. Every populated structural record in all 17 examples is
represented, and the source record remains attached as `lpf_raw` while interpreter parameter
slots are being assigned their final protocol-specific names. SampleData is preserved as RLE
and is tested bit-for-bit at every source-run boundary without expanding large captures.

D6 is not complete until the protocol-specific interpreter slot semantics and remaining trigger
settings are fully typed and the 17 approved visual snapshots are stable.
