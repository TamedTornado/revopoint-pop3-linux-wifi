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
value 3 as `Z16Y8Y8` and format value 4 as `PAIR`. Z16Y8Y8 contains a Z16 plane
followed by left and right Y8 planes; PAIR contains only the two Y8 planes.

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
contiguous right Y8 plane.

Android library inspection maps `Z16Y8Y8` to selector 3 and a four-byte display
profile; the display type is bytes per pixel, not the stream-format enum itself.
A patched temporary copy of Revopoint's obsolete Linux SDK was used only as a
black-box network oracle after its host-side license gate rejected this newer
scanner. System-call tracing exposed two setup operations missing from the
first clean-room probe: close all existing streams before changing the profile,
and set trigger mode 0 for free-running acquisition. No vendor library, patch,
or captured frame is present in this repository.

Replaying the independently observed sequence in Rust—close streams, select a
640×400 four-byte display profile, select output 3, set free-running trigger,
then open camera 21—produced live media without rebooting the scanner. The
decoded 1,024,000-byte frame splits exactly into 512,000 bytes of little-endian
Z16 and two 256,000-byte Y8 planes. The acquisition path retains PAIR as a
separate diagnostic mode rather than conflating its intensities with depth.

The first 80 bytes of each decompressed depth buffer are a packed auxiliary
record, not depth samples. Binary inspection established that the SDK copies
that prefix out before zeroing it in the image buffer. Its little-endian device
timestamp is at byte offset 20. Four consecutive saved frames contained
208,743, 208,790, 208,837, and 208,884 ms: an exact 47 ms cadence. The Rust
decoder now extracts that timestamp and likewise clears all 80 metadata bytes
before exposing the Z16 plane, preserving the vendor's pixel indexing without
mistaking metadata for the first 40 depth pixels.

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
A separately distributed Android Revo Scan 5.3.4 artifact (SHA-256
`20d55e97c553a25fcafde0a486c2dd08040cc40106a6e5b5ef949acf2ee74c25`)
provided an independent clean-room cross-check: its exported rectification
helper reads the same intrinsic and Brown–Conrady fields, removes distortion
iteratively for the forward coordinate query, and applies the inverse of the
stored final 3×3 transform. This corroborates the Rust inverse-map direction
and coefficient order without incorporating vendor code. The image resampler
now uses bilinear interpolation rather than nearest-neighbor rounding; all 50
mutants in that change were killed by focused tests.

A whole-image horizontal comparison of the rectified wall target had its lowest
error at approximately 14–15 half-resolution pixels, providing an independent
sanity check that the epipolar direction is horizontal. Per-pixel SAD disparity
is filtered by a best-versus-runner-up cost margin, checked against a reverse
right-to-left match, and emitted for diagnosis. The nearly planar, low-texture
target remains a deliberately difficult case and the output is not treated as
qualified depth.

On a later live wall capture, bilinear rectification retained 38,697 of 256,000
pixels (15.1%), with a global disparity of 20 pixels, provisional median depth
of 264 mm, median absolute deviation of 8 mm, and a 233–272 mm p10–p90 range.
That is only a marginal retention change from nearest-neighbor rectification;
the disparity image remains fragmented, so the result validates the resampling
boundary but still does not qualify the stereo geometry.

The accompanying 4×4 Q record follows the standard homogeneous reprojection
shape. Scaling half-resolution disparity back to calibration resolution and
applying the complete homogeneous transform per pixel produced a 260 mm median
after provisional uniqueness and consistency filtering. That run retained
53,113 of 256,000 pixels (20.7%); the wall's physical distance was estimated
only as 150–300 mm. The broad agreement supports the interpretation but is
intentionally not a metric acceptance result.

The smoke tool also serializes this experimental metric plane as a 16-bit PGM:
samples are unsigned millimeters in network byte order, as required by PGM,
with zero reserved for invalid correspondence. ImageMagick identifies a live
artifact as 640×400, 16-bit, with a 0–272 sample range.

The scanner's read-only `get_depth_reso` endpoint reports `curr-resolution`
according to the active profile. Under PAIR it reported `640x400x2`, which is
also the total byte count for two Y8 planes and was not evidence of Z16. Under
the Z16Y8Y8 profile it reports `640x400x4`, matching the verified four-byte
layout.

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
neighboring intensity pixels. These observations invalidate the earlier
selector-1 depth claim.

A later selector-3 Z16Y8Y8 run captured and decoded scanner-computed depth at
640×400. It contained 163,844 nonzero samples (64.0%). Applying the independently
queried 0.1 mm scale yielded a 167.8 mm median, 1.5 mm median absolute deviation,
and 165.0–170.7 mm p10–p90 range while the scanner was roughly 150–300 mm from a
wall. The accompanying left infrared image showed the same wall boundary and
projected texture. This qualifies the acquisition and plane-splitting slice;
it does not yet qualify absolute accuracy because the target distance was not
measured for that run.

### Depth exposure controls

Static inspection of the distributed SDK and its public headers identifies
register `0x911` as manual depth exposure in microseconds, `0x912` as the depth
auto-exposure mode, and `0x910` as frame time. The public viewer sets frame time
to exposure plus 2,000 microseconds before setting manual exposure. The four
documented automatic values are off (0), fixed frame time (1), high quality
(2), and foreground priority (3). The network camera accepts the same register
writes through its existing `system_cmd` CGI endpoint.

The POP 3 used for live validation reported a manual exposure range of
5,000–65,000 microseconds in one-microsecond steps. A scanning-spray trial made
the need for explicit control visible: inherited state yielded only 73 colored
points and almost completely white infrared frames. Setting 7,000 microseconds
raised scanner-computed valid depth to 16,910 pixels; setting the reported
5,000-microsecond minimum raised it to 45,249 pixels, and the paired RGB-D path
exported 45,038 colored points with a 21 ms timestamp delta. Foreground auto
exposure retained essentially the same result (45,329 valid pixels). These are
acquisition-yield observations, not geometric-accuracy qualification.

The Rust API models these controls as enums and validated values. Capture CLI
paths explicitly select foreground auto exposure unless the caller supplies a
manual exposure or another auto mode, eliminating dependence on whatever a
previous client left in persistent camera state.

The completed Rust path was then exercised directly, without an external
`curl` setup step. An explicit 5,000-microsecond RGB-D capture produced 72,684
colored points. A subsequent default foreground-auto capture produced 102,121
colored points with a 24 ms paired timestamp delta. This validates that both
manual and default automatic controls are applied inside acquisition.
