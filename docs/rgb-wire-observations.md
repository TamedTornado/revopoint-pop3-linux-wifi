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
were consistently about 14–15 ms apart in the observed portion. The clean-room
driver does not yet implement or qualify that pairing policy.

Three read-only calibration files were also recovered through the existing
download command:

- `Prgb.bin`: 40-byte calibration resolution plus 3×3 RGB camera matrix;
- `Distort.bin`: five little-endian `f32` RGB distortion coefficients;
- `LC_RT.bin`: nine little-endian `f32` rotation values followed by a
  three-element millimetre translation from the left depth camera to RGB.

The Rust parser validates all three layouts and the live POP 3 Plus files. The
exact registration/resampling policy and paired-frame acceptance tolerance
remain to be qualified.
