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

The official binary configures network profiles with explicit display width,
height, and stream-format values. Revopoint's public SDK headers identify format
value 2 as Z16 and format value 4 as `PAIR`, containing left and right Y8 images.

The Windows SDK's network start path changes that selection in place; it does
not power-cycle the scanner. It sends the profile, waits 300 ms, and retries the
output-selector request at most three times before startup fails.

Inspection of RevoScan's own `start depth stream` call site establishes the
important missing distinction: the application passes stream format `4`
(`PAIR`), width 640, and height 480 to its camera-management wrapper. That
wrapper obtains the camera's available stream descriptions and calls the camera
interface's `startStream` virtual method with the selected PAIR description. In
other words, RevoScan itself does not ask this network camera for Z16 at the
acquisition boundary; its later processing pipeline derives depth from the
binocular images.

The camera library's later algorithm service confirms that separation. It logs
left, right, match, and left/right-consistency error rates and dynamically loads
`3dcameraAlg.dll` through the `algSdk.algLibPath` setting before creating its
processing object. That model-specific dense algorithm DLL was not present in
the unpacked RevoScan application examined here. This project therefore treats
rectification, correspondence, consistency, and reprojection as an explicit
clean-room boundary rather than copying vendor implementation code.

Selector 2 is accepted but produced no bytes from `camera_id=21` in isolated
hardware probes. That is no longer assumed to indicate a missing selector
prerequisite: it may simply be an unused firmware path for this model.

Selector 1 reliably produces media without a power cycle. Current binary
inspection maps the SDK's `PAIR` format to selector 1, and the public SDK source
defines its memory layout as one contiguous left Y8 plane followed by one
contiguous right Y8 plane. The Rust acquisition path now exposes only that
verified PAIR result.

A usable PAIR image also requires both pre-ISP illumination controls: register
`0xb00` enables the LED master and `0xb01` enables the infrared projector. With
the selector configured but those controls off, frames were nearly black. With
both enabled and a 300 ms settling delay, the same bounded capture produced two
bright, recognizable views of the physical target. The client disables the
projector and master after every attempted capture; a fixture-server integration
test also verifies cleanup when projector enablement is rejected.

The scanner exposes 148-byte `mapparamL.bin` and `mapparamR.bin` calibration
records. Each begins with calibration height, width, and a five-coefficient
distortion count; it then contains camera intrinsics, Brown–Conrady distortion,
and a 3×3 inverse rectification transform. Applying those records to a live
640×400 PAIR capture produced the expected small black rectification borders.
A whole-image horizontal comparison of the rectified wall target had its lowest
error at approximately 14–15 half-resolution pixels, providing an independent
sanity check that the epipolar direction is horizontal. Per-pixel SAD disparity
is now emitted for diagnosis but remains visibly noisy on the nearly planar,
low-texture target and is not treated as qualified depth.

The accompanying 4×4 Q record follows the standard homogeneous reprojection
shape. Scaling half-resolution disparity back to calibration resolution and
dividing Q's Z numerator by its homogeneous W term produced a 250.5 mm median
on a live wall capture whose physical distance was estimated only as 150–300
mm. The broad agreement supports the interpretation but is intentionally not a
metric acceptance result.

The scanner's read-only `get_depth_reso` endpoint reports `curr-resolution` as
`640x400x2`. That is also the total byte count for two Y8 planes: 512,000 bytes.
It is not sufficient evidence of a little-endian Z16 plane.

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

Selector 1 produced 512,000-byte decoded frames. Interpreting each adjacent byte
pair as a `u16` yielded values around 190 mm and a non-empty RViz cloud, but that
interpretation was wrong. The bytes are contiguous left/right Y8 planes. The
apparently geometric V-shaped cloud was an artifact of combining unrelated
neighboring intensity pixels. These observations invalidate the earlier live
Z16, metric-depth, and RViz acceptance claims; the repository now fails closed
instead of exposing them.
