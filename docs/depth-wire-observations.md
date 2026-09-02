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

The official binary configures network depth profiles with explicit display
width, height, and stream-format values. Revopoint's public SDK headers identify
format value 2 as Z16: one unsigned 16-bit depth value per pixel. The client now
selects 640x400 Z16 explicitly before opening media rather than relying on the
scanner's previous state.

The scanner's read-only `get_depth_reso` endpoint then reports
`curr-resolution` as `640x400x2`. The client parses that as width 640, height
400, two bytes per pixel, a 1,280-byte stride, and a 512,000-byte frame. Every
hardware-decoded frame must match that layout exactly before becoming an owned
little-endian Z16 plane.

The official SDK describes its depth scale as the physical millimeters
represented by one raw depth unit. Binary inspection established that the
network implementation reads property command `0x918` and calculates the scale
as one divided by the returned integer. The scanner's read-only response
returned divisor 10, so this device's observed scale is 0.1 mm per raw unit.

The scanner's read-only calibration download returns a 40-byte depth
intrinsics record: little-endian 16-bit calibration width and height followed
by a row-major 3x3 `f32` pinhole matrix. The client requires the canonical
zero/skew fields and bottom row before accepting the record. The tested device
reported calibration dimensions 1280x800, `fx=fy=1747.4467`, `cx=257.196`, and
`cy=402.5506`.

Revopoint's public SDK point projection scales `fx` and `cx` by stream width
over calibration width, and `fy` and `cy` by stream height over calibration
height. Applying that published transform to the selected 640x400 stream gives
`fx=fy=873.7233`, `cx=128.598`, and `cy=201.2753`. The Rust parser, validation,
scaling, and read-only HTTP boundary are mutation-clean. Distortion,
rectification state, optical-frame publication, and physical-target validation
remain open.

## Current hardware receipt

On 2026-09-02, firmware `v3.2.36.20241219` repeatedly produced four complete
envelopes inside a bounded 1 MiB capture. Every complete payload decompressed to
exactly 1,024,000 bytes; one representative run had compressed sizes 233,528,
234,171, 238,701, and 238,274 bytes. The fifth frame was intentionally cut off
by the acquisition limit and was not counted.

After explicit Z16 selection, four further hardware frames each decompressed to
exactly 512,000 bytes. One bounded run found 255,997 nonzero samples per
256,000-pixel frame and mean nonzero depths from 190.47 to 190.76 mm using the
device-reported scale. Minimum and maximum raw values are retained as diagnostic
receipts but are not yet treated as validated range limits.

A subsequent bounded smoke downloaded the 1280x800 calibration record and
reported the expected scaled 640x400 intrinsics alongside four valid Z16 plane
receipts.
