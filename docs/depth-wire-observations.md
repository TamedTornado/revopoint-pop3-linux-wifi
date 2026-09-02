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

## Compressed payload

The envelope payload is a QuickLZ 1.5 stream with a nine-byte long header. On
the tested scanner, byte zero is `0x47`; the following little-endian words are
the compressed and decompressed lengths. The compressed length repeats the
outer-envelope length exactly.

The Rust decoder checks both length declarations and applies a caller-provided
output ceiling before decompression. An independent, project-owned QuickLZ
fixture covers the codec boundary; the repository contains no captured scanner
frames.

Inspection of the official binary corroborated the algorithm rather than
supplying an implementation: the depth decoder identifies itself as
`DecoderDepthQuicklz`, compares QuickLZ's compressed-size result with the bytes
available, and sizes its destination as width times height times format bits,
rounded to bytes. The project uses the independently published `quicklz` Rust
crate rather than copying or translating that routine.

The scanner's read-only `get_depth_reso` endpoint reported
`curr-resolution` as `640x800x2` during the same session. Its full response also
advertised separate depth, calibration, depth-plus-IR, and depth-plus-IR-plus-P
profiles. We have not yet assigned those components to the decompressed plane or
claimed a metric pixel interpretation.

## Current hardware receipt

On 2026-09-02, firmware `v3.2.36.20241219` repeatedly produced four complete
envelopes inside a bounded 1 MiB capture. Every complete payload decompressed to
exactly 1,024,000 bytes; one representative run had compressed sizes 233,528,
234,171, 238,701, and 238,274 bytes. The fifth frame was intentionally cut off
by the acquisition limit and was not counted.
