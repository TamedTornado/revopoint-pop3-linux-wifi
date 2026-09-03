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
and contains a baseline 1280×800 image. Four bytes follow the JPEG EOI marker
inside the declared envelope payload. Their meaning is not established, so the
Rust boundary exposes only their observed size and strips them when writing the
ordinary JPEG; it does not label them as a timestamp or sequence number.

The scanner does not emit RGB media merely because camera 50 is opened. A
bounded negative probe received zero bytes until free-running trigger mode was
set. Repeating the same profile and endpoint with that command produced roughly
2.5 MB in ten seconds. The Rust smoke command subsequently captured a bounded
1 MiB prefix, recovered a complete frame, validated its JPEG dimensions, and
wrote a recognizable image of the physical wall target.

RGB capture is currently independent of depth capture. There is not yet evidence
for temporal synchronization, depth-to-color extrinsics, registration state, or
the meaning of the four-byte trailer.
