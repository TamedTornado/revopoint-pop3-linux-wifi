# Depth wire observations

This document records the minimum independently verified facts needed for Linux
interoperability. It is not a copy of a vendor header, source file, binary, or
decompiler output.

## Outer frame envelope

The decoded HTTP chunk body begins with a four-byte preamble (`0d 0a 0d 0a`) on
the tested POP 3 Plus, followed by repeated frames:

| Offset | Width | Established meaning |
|---:|---:|---|
| 0 | 4 | little-endian magic `0x11223344` |
| 4 | 4 | little-endian payload byte length |
| 8 | declared length | opaque compressed frame payload |

In an early 1,264,550-byte hardware capture, magic appeared at offsets 4,
315751, 632295, and 948887. The first three declared lengths were 315739,
316536, and 316584 bytes. In every complete case, `start + 8 + length` equaled
the next magic offset exactly.

Inspection of the official Revo Scan `3DCamera.dll` independently corroborated
the eight-byte contract: its outer parser requires eight available bytes,
compares the first word with `0x11223344`, reads the second word as the length,
and returns eight as the consumed header size. One observed variant also rejects
non-positive lengths.

No meaning is currently assigned to fields inside the compressed payload. In
particular, the nearby decoded-frame metadata validator in the SDK binary must
not be mistaken for the on-wire compressed layout.

## Current hardware receipt

On 2026-09-02, firmware `v3.2.36.20241219` produced four complete envelopes
inside a bounded 1 MiB capture, with payload sizes 234111, 235358, 238437, and
233730 bytes. The fifth frame was intentionally cut off by the acquisition
limit and was not counted.
