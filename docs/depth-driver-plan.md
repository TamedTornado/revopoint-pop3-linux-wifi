# Calibrated depth to RViz: implementation plan

## Goal

Produce a live, calibrated metric depth stream from a Revopoint POP 3 Plus on
Linux, publish it through standard ROS 2 camera topics, and inspect the depth
image and a stock `depth_image_proc` point cloud in RViz2.

The initial product scenario is a stationary scanner observing a small object
on a turntable. Multi-view reconstruction is deliberately outside this first
milestone.

## Boundaries

- Rust implementation; no CMake project for our code.
- Independent interoperability work only. Do not commit vendor source, SDK
  headers, binaries, firmware, or application assets.
- The acquisition core must not depend on ROS.
- ROS is an adapter over calibrated frames, not the internal data model.
- Point clouds are derived by standard ROS tooling for this milestone.
- A fixture pass cannot be presented as hardware qualification.
- Dependency resolution uses a date-pinned Cargo nightly with native
  `min-publish-age` enforcement and a global seven-day publication quarantine.
  Exceptions require Jason's explicit authorization.

## Delivery sequence

### 0. Preserve the known Wi-Fi baseline

The verified Wi-Fi client provisioning path is committed independently before
depth work begins. Its tests remain part of every gate.

### 1. Establish test and evidence structure ([#1](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/1))

Create a reusable library boundary, deterministic unit tests, offline
integration tests through public process/network boundaries, a small
qualification matrix, and Ubuntu 24.04 CI. Hardware-only cells remain visibly
unclaimed by CI.

### 1a. Enforce dependency publication age ([#10](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/10))

Pin the reviewed Cargo nightly, enable its native publication-age resolver,
deny incompatible ages, and set a global seven-day floor. Retain an empty,
explicit exception manifest unless Jason authorizes an exact exception. Audit
the resulting lockfile separately as defense in depth.

### 2. Acquire a bounded network stream ([#2](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/2))

Replace the exploratory `curl` command with a bounded Rust HTTP stream client.
Test fragmentation, timeout, truncation, non-success responses, limits, and
clean close behavior against a local server. Retain an opt-in real-scanner
smoke command.

Completed acquisition evidence: offline fixtures cover fragmented chunked and
Content-Length responses, truncation, timeout, bounds, and cleanup after error.
Five consecutive hardware smokes each captured exactly 1 MiB and successfully
closed and restarted the scanner stream. This does not claim complete frames;
frame recovery remains phase 3.

### 3. Recover complete frame envelopes ([#3](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/3))

Identify only the wire fields required to delimit and validate frames. Test
every header split, concatenated frames, corruption, truncation, and size
limits before trusting hardware frame counts.

The independently observed and binary-corroborated outer envelope is implemented
and mutation-clean. Hardware runs recovered complete compressed PAIR and
Z16Y8Y8 frames. The observed eight-byte envelope contains no dimensions or
continuity field; those properties are validated at the decoded profile layer
rather than assigned to unknown bytes.

### 4. Decode the depth plane ([#4](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/4))

Decode compressed payloads in clean-room Rust using project-owned fixtures.
Return an explicit width, height, stride, encoding, and pixel buffer. Reject
malformed data without panics or unbounded allocation.

Completed status: QuickLZ and PAIR decoding work, and the distinct selector-3
`Z16Y8Y8` path now yields scanner-computed depth. Tracing the obsolete vendor
SDK as a black-box network oracle exposed the missing close-stream and
free-running-trigger operations. The clean-room Rust path replays those
commands, validates the 1,024,000-byte decoded layout, and returns explicit
640×400 width, height, stride, little-endian Z16 encoding, 0.1 mm scale, and
separate left/right Y8 planes. No vendor code or capture is committed.

First reconstruction slice: the 148-byte left/right map-parameter records are
parsed and validated in clean-room Rust. The Y8 planes can be rectified using
their Brown–Conrady coefficients and inverse rectification matrices. A bounded
block matcher with a provisional uniqueness margin and left/right consistency
emits experimental disparity and 16-bit millimeter-depth PGM images for
immediate inspection in a stock Linux viewer. The scanner's Q matrix is parsed
and applied per pixel with the calibration-to-stream coordinate and disparity
scales. A confidence-filtered live wall run kept
53,113 of 256,000 pixels (20.7%) and returned an experimental median of 259.8 mm
while the target was known only to be roughly 150–300 mm away. This is a sanity
check, not yet accepted metric depth.

After RViz exposed separated lobes instead of a plane, a mutation-clean 5×5
spatial-consensus/median filter and robust depth statistics were added. A live
filtered run kept 37,655 pixels (14.7%), with median 263 mm, median absolute
deviation 9 mm, and a 234–272 mm 10th–90th percentile interval. That is a
quantified failure on a roughly planar target, not a promoted result.

An independent whole-frame normalized SAD sweep now reports the dominant shift
beside the per-pixel result. On a subsequent capture it found 20 px (258.5 mm)
while the filtered local median was 11 px (264 mm) with an 8 mm MAD. The correct
sign and plausible global peak narrow the remaining problem to ambiguous local
correspondence/regularization rather than another selector or byte-layout guess.

### 5. Establish metric meaning and calibration ([#5](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/5))

Determine depth scale, invalid values, intrinsics, distortion, optical-frame
convention, and rectification state. Validate scale and planar error against
measured physical targets. The output of this phase is a calibrated depth
frame, not merely decoded integers.

The device depth-unit divisor and read-only depth intrinsics can be parsed and
scaled. Direct Z16Y8Y8 acquisition now applies the observed 0.1 mm unit scale;
a live wall run reported 167.8 mm median depth and 1.5 mm MAD. Issue #5 remains
open because the wall distance was only estimated at 150–300 mm: measured
targets at multiple distances and optical-frame/rectification qualification are
still required.

### 5a. Mutation-test deterministic contracts ([#9](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/9))

Run a pinned Rust mutation tool over HTTP framing, frame parsing,
decompression bounds, calibration conversion, and projection. Every viable
survivor becomes a red test before it is fixed. Keep mutation claims separate
from network, hardware, ROS, and visual qualification.

### 6. Publish ROS 2 camera messages ([#6](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/6))

Add a feature-gated Rust ROS 2 Jazzy adapter publishing synchronized
`sensor_msgs/Image` and `sensor_msgs/CameraInfo` messages with sensor-data QoS.
Test exact message fields using a real ROS subscriber boundary.

The offline Z16-to-ROS mapping contract remains tested. Live publication now
uses the scanner-computed Z16Y8Y8 plane rather than the experimental PAIR
reconstruction. A bounded hardware run published all 20 requested frames; a
real ROS subscriber received a 640-wide `32FC1` image. This validates the live
message boundary but not metric accuracy.

### 7. Qualify the stock Linux application path ([#7](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/7))

Feed the topics to `depth_image_proc::PointCloudXyzNode`, display the live
image and derived cloud in RViz2, and record automated graph evidence plus a
real-hardware acceptance receipt. Exercise turntable motion, scale,
orientation, shutdown, and restart.

Current status: scanner-computed Z16Y8Y8 now drives stock `depth_image_proc`, and
a subscriber received a 640-wide organized `PointCloud2`. RViz2 launched with
the supplied configuration and subscribed to the direct-depth cloud after a
static optical-frame transform was supplied. Wayland denied automated screenshot
capture, and measured plane/motion checks still require manual evidence, so this
phase remains partial rather than complete.

### 8. Continue into turntable RGB-D capture ([#8](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/8))

In progress: a black-box trace identified the 1280×800 RGB profile, RGB sensor
enable control, free-running trigger requirement, and camera-50 media endpoint.
The clean-room Rust path now recovers envelope-delimited JPEG frames, validates
their dimensions, separates their little-endian millisecond timestamp, and
writes an ordinary JPEG. A live smoke produced a recognizable 1280×800 wall
image. A public Rust command now holds the depth and RGB endpoints open together
until both have reached complete frame boundaries, then writes checkable depth
PGM and RGB JPEG files. The path also downloads and validates RGB intrinsics,
five distortion coefficients, and the left-depth-camera-to-RGB transform.
These frames are concurrent but not yet paired. Pairing policy, registration,
archive format, turntable angle metadata, masking, and dataset adapters remain
open.

## Red-green workflow

For each behavior:

1. Add the smallest failing unit or integration test that states the contract.
2. Capture the failing command and reason in the issue or commit context.
3. Implement only enough behavior to pass it.
4. Run the focused test, then its containing tier, then formatting and clippy.
5. For hardware changes, run the opt-in scanner cell and record its distinct
   outcome; never substitute fixture evidence.

## Test tiers

| Tier | Boundary | Normal command | Public CI |
|---|---|---|---|
| Unit | Pure parser, decoder, calibration, and mapping logic | `cargo test --lib` | Required |
| Offline integration | Built CLI and local HTTP fixture server | `cargo test --test '*'` | Required |
| Mutation | Deterministic protocol, decoder, and calibration contracts | milestone command | Bounded subset later |
| ROS integration | ROS publishers/subscribers and `depth_image_proc` | milestone command | Later CI job |
| Hardware | Real POP 3 on the LAN | opt-in ignored test/tool | Local only |
| Visual acceptance | Real hardware and RViz2 | documented session | Manual only |

The authoritative current claim status is in `qualification-matrix.json`.

## Milestone exit criteria

- A stock RViz2 session shows continuously updating depth imagery.
- Stock `depth_image_proc` produces a non-empty organized point cloud.
- Known distances and planar geometry meet recorded tolerances.
- Disconnect, error, shutdown, and restart are bounded and observable.
- Unit, offline integration, ROS integration, hardware, and visual evidence are
  reported separately and agree with the qualification matrix.
