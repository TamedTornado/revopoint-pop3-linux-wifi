# RGB wire observations

This document records only the independently observed fields required for
Linux interoperability. No vendor source, binary, or captured frame is stored
in this repository.

The network RGB start sequence observed on the tested POP 3 Plus is:

1. close existing streams;
2. select the USB-camera 1280×800 profile;
3. enable the shared LED master and RGB sensor controls;
4. select free-running trigger mode;
5. open media camera 50 with type 20.

The media body uses the same `0x11223344` little-endian magic and payload-length
envelope as the depth stream. Each observed RGB payload starts with a JFIF JPEG
and contains a baseline 1280×800 image. A little-endian `u32` follows the JPEG
EOI marker inside the declared envelope payload. The vendor SDK's published
header describes frame timestamps as milliseconds. Across one 44-frame direct
capture, the field increased monotonically by 9,305 over approximately ten
seconds; a later black-box SDK run returned values of the same magnitude and
cadence from `frameGetTimestamp`. The clean-room boundary therefore exposes it
as the device millisecond timestamp and excludes it from the ordinary JPEG.

The scanner does not emit RGB media merely because camera 50 is opened. A
bounded negative probe received zero bytes until free-running trigger mode was
set. Repeating the same profile and endpoint with that command produced roughly
2.5 MB in ten seconds. The Rust smoke command subsequently captured a bounded
1 MiB prefix, recovered a complete frame, validated its JPEG dimensions, and
wrote a recognizable image of the physical wall target.

The scanner can serve depth camera 21 and RGB camera 50 at the same time. A
bounded six-second direct probe recovered 23 complete depth envelopes and 26
complete RGB envelopes. The vendor SDK also starts the two streams separately
and exposes a `getPairedFrame` operation; a black-box callback run returned 36
depth frames and 35 RGB frames in eight seconds. Nearest depth/RGB SDK timestamps
were consistently about 14–15 ms apart in the observed portion.

The clean-room Rust concurrent smoke now configures both sensors, opens both
media endpoints together, and waits until both parsers have complete frame
boundaries before closing either connection. A live run recovered 647,121 depth
bytes and 115,879 RGB bytes, decoded a 640×400 Z16Y8Y8 frame, and wrote a
validated 1280×800 JPEG with device timestamp 22,575,391 ms. Both ordinary image
files were visually checked. An earlier fixed-prefix attempt consistently let
the faster depth capture close first and left the slower RGB capture silent;
coordinating on application-frame completion fixed that transport-lifetime
error without a scanner power cycle.

Depth binary inspection then located a timestamp in the decompressed depth
metadata prefix. A new live concurrent run reported depth timestamp 23,709,703
ms and RGB timestamp 23,709,718 ms, establishing that both fields use the same
device uptime clock and reproducing the 15 ms separation seen through the SDK.
The smoke command now collects eight frames from each stream, finds the closest
pair for which RGB follows depth within an explicit 50 ms window, and handles
`u32` timestamp wraparound. The first one-frame attempt correctly rejected a
171 ms startup-skewed pair; the multi-frame run then selected a 15 ms pair from
5,035,118 depth transport bytes and 505,455 RGB transport bytes.

Three read-only calibration files were also recovered through the existing
download command:

- `Prgb.bin`: 40-byte calibration resolution plus 3×3 RGB camera matrix;
- `Distort.bin`: five little-endian `f32` RGB distortion coefficients;
- `LC_RT.bin`: nine little-endian `f32` rotation values followed by a
  three-element millimetre translation from the left depth camera to RGB.

The Rust parser validates all three layouts and the live POP 3 Plus files.
Revopoint's public SDK processing header establishes its point-color projection
convention: add the three translation values to a point in the left depth
camera frame, multiply by the stored 3×3 rotation in row-major order, then
project with the RGB pinhole matrix. That published point generator does not
apply the separately reported distortion coefficients during this operation.
The clean-room Rust implementation follows that convention and truncates the
projected coordinates for nearest-pixel RGB sampling.

A corrected live run selected another 15 ms pair, decoded the JPEG without a
vendor codec, registered 163,174 valid depth samples to RGB, and wrote a 2.4 MB
binary little-endian colored PLY. Ubuntu's stock CloudCompare 2.11.3 loaded the
file as one cloud with exactly 163,174 points and successfully computed normals.
The wall target is too visually uniform to qualify pixel-level color alignment,
so a target with recognizable depth and color edges remains required.
