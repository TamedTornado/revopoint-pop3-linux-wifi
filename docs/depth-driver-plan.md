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
is implemented and mutation-clean. A 1 MiB hardware run recovered four complete
compressed frames. Inner dimensions and continuity fields are not yet
established, so this phase and issue remain open.

### 4. Decode the depth plane ([#4](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/4))

Decode compressed payloads in clean-room Rust using project-owned fixtures.
Return an explicit width, height, stride, encoding, and pixel buffer. Reject
malformed data without panics or unbounded allocation.

### 5. Establish metric meaning and calibration ([#5](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/5))

Determine depth scale, invalid values, intrinsics, distortion, optical-frame
convention, and rectification state. Validate scale and planar error against
measured physical targets. The output of this phase is a calibrated depth
frame, not merely decoded integers.

In progress: the device depth-unit divisor and read-only depth intrinsics are
parsed, validated, scaled to the selected 640x400 stream using the public SDK's
documented projection transform, exercised against hardware, and
mutation-clean. Distortion/rectification semantics, optical-frame publication,
and physical-target qualification remain open.

### 5a. Mutation-test deterministic contracts ([#9](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/9))

Run a pinned Rust mutation tool over HTTP framing, frame parsing,
decompression bounds, calibration conversion, and projection. Every viable
survivor becomes a red test before it is fixed. Keep mutation claims separate
from network, hardware, ROS, and visual qualification.

### 6. Publish ROS 2 camera messages ([#6](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/6))

Add a feature-gated Rust ROS 2 Jazzy adapter publishing synchronized
`sensor_msgs/Image` and `sensor_msgs/CameraInfo` messages with sensor-data QoS.
Test exact message fields using a real ROS subscriber boundary.

In progress: a ROS-independent mapping contract converts device-scaled Z16 to
standard `32FC1` meters and constructs synchronized rectified camera metadata.
The mapping is mutation-clean. A feature-gated rclrs 0.7 adapter now creates
real runtime-typed Jazzy `sensor_msgs/Image` and `CameraInfo` messages under
sensor-data QoS; the installed type-support integration tests pass. Live
publishing now captures 20 bounded batches and a stock Jazzy subscriber received
the resulting runtime-typed topics. Device sequence/timestamp recovery remains
open; the provisional publisher uses a shared host timestamp per message pair.

### 7. Qualify the stock Linux application path ([#7](https://github.com/TamedTornado/revopoint-pop3-linux-wifi/issues/7))

Feed the topics to `depth_image_proc::PointCloudXyzNode`, display the live
image and derived cloud in RViz2, and record automated graph evidence plus a
real-hardware acceptance receipt. Exercise turntable motion, scale,
orientation, shutdown, and restart.

In progress: stock Jazzy `PointCloudXyzNode` synchronized the live Image and
CameraInfo topics and emitted a non-empty organized 640x400 `PointCloud2`.
RViz2 visual acceptance, turntable motion, planar-target measurement, and
restart qualification remain open.

First visual receipt: RViz2 loaded the repository profile and rendered the live
organized cloud against its metric grid. The initial screenshot is an oblique,
nearly edge-on view through the scanner frustum, so it proves non-empty spatial
geometry rather than recognizable-object shape or metric accuracy. Those
stronger claims still require a known target and deliberate viewing angles.

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
