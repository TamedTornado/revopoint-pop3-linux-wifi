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

In progress: the independently observed and binary-corroborated outer envelope
is implemented and mutation-clean. Hardware runs recovered complete compressed
PAIR frames. Inner dimensions and continuity fields are not yet established.

### 4. Decode the depth plane ([#4](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/4))

Decode compressed payloads in clean-room Rust using project-owned fixtures.
Return an explicit width, height, stride, encoding, and pixel buffer. Reject
malformed data without panics or unbounded allocation.

Corrected status: QuickLZ decoding works, but the live selector is `PAIR`, not
Z16. Its two contiguous Y8 planes are now decoded explicitly. RevoScan's own
`start depth stream` call site passes format 4 (`PAIR`) into the camera API,
confirming that metric depth is derived later on the host. Issue #4 must remain
open until that stereo reconstruction boundary is independently implemented.

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

### 5. Establish metric meaning and calibration ([#5](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/5))

Determine depth scale, invalid values, intrinsics, distortion, optical-frame
convention, and rectification state. Validate scale and planar error against
measured physical targets. The output of this phase is a calibrated depth
frame, not merely decoded integers.

The device depth-unit divisor and read-only depth intrinsics can be parsed and
scaled, but they cannot be applied directly to PAIR intensities. All live metric
claims are withdrawn pending independently implemented rectification, disparity,
and depth reconstruction from the verified binocular input.

### 5a. Mutation-test deterministic contracts ([#9](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/9))

Run a pinned Rust mutation tool over HTTP framing, frame parsing,
decompression bounds, calibration conversion, and projection. Every viable
survivor becomes a red test before it is fixed. Keep mutation claims separate
from network, hardware, ROS, and visual qualification.

### 6. Publish ROS 2 camera messages ([#6](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/6))

Add a feature-gated Rust ROS 2 Jazzy adapter publishing synchronized
`sensor_msgs/Image` and `sensor_msgs/CameraInfo` messages with sensor-data QoS.
Test exact message fields using a real ROS subscriber boundary.

The offline Z16-to-ROS mapping contract remains tested. Live publication is now
restored only for the independently reconstructed plane and is labelled
experimental. A bounded hardware run published all 20 requested frames; a real
ROS subscriber received a 640-wide `32FC1` image. This validates the live
message boundary but not metric accuracy.

### 7. Qualify the stock Linux application path ([#7](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/7))

Feed the topics to `depth_image_proc::PointCloudXyzNode`, display the live
image and derived cloud in RViz2, and record automated graph evidence plus a
real-hardware acceptance receipt. Exercise turntable motion, scale,
orientation, shutdown, and restart.

Corrected status: the ROS graph and RViz plumbing worked, but its live input was
not depth. The V-shaped cloud was a useful falsification signal, not an
acceptance receipt. The corrected reconstruction now drives stock
`depth_image_proc`: a subscriber received a 640-wide organized `PointCloud2`,
and RViz2 displayed the live reconstructed depth image and cloud after a static
optical-frame transform was supplied. The roughly planar wall appeared as sparse
separated lobes, so the visual geometry check failed usefully. This phase
remains partial rather than complete.

### 8. Continue into turntable RGB-D capture ([#8](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/8))

After the depth milestone, add RGB synchronization, cross-camera calibration,
foreground masks, known turntable angles, replayable capture archives, and
dataset adapters. Reconstruction software is selected using those observed
captures rather than assumed compatibility.

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
