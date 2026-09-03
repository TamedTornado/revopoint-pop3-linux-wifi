# Revopoint POP 3 Linux interoperability

A Linux-native Rust project for interoperating with a Revopoint POP 3 scanner.
It currently provisions Wi-Fi client credentials over USB and performs bounded
depth, infrared, and RGB acquisition over the LAN. It avoids installing or
running Revo Scan, Windows, Wine, or a virtual machine.

This project is an independent interoperability implementation. It contains no
Revopoint source code, SDK headers, binaries, firmware, application assets, or
credentials.

## Current status

Tested on:

- Revopoint POP 3 Plus, USB ID `2207:110c`
- scanner firmware `v3.2.36.20241219`
- Ubuntu 24.04, x86-64

The read path has been tested against real hardware. The write path:

1. prompts for the SSID and password locally;
2. hides password input and never puts it in process arguments;
3. validates WPA2 credential lengths;
4. writes the scanner's client configuration;
5. disables the scanner's own access point while preserving its existing AP
   configuration;
6. reads both files back and requires byte-for-byte matches; and
7. asks the scanner to sync the filesystem.

It does **not** change firmware or send an upgrade command.

The network acquisition path can configure either the scanner's `PAIR` output
or its device-computed `Z16Y8Y8` output, capture an exact byte count from the
chunked HTTP media stream, recover bounded frame envelopes, and decompress the
QuickLZ payloads in Rust. PAIR frames split into two 640x400 Y8 infrared images;
Z16Y8Y8 frames split into a little-endian Z16 plane followed by the two Y8
planes.

Physical accuracy is **not yet qualified**. Hardware tests disproved an earlier
interpretation of selector 1 as Z16: those bytes are the two Y8 images described
by the public SDK's `PAIR` layout. Treating adjacent bytes as little-endian
depth created a plausible-looking but false point cloud. Selector 3 is now
independently identified as `Z16Y8Y8` and produces a distinct, scanner-computed
depth plane after the vendor-observed stream-reset and free-running-trigger
sequence. A live wall capture reported a 167.8 mm median with 1.5 mm median
absolute deviation while the scanner was estimated to be 150–300 mm away. That
is a strong acquisition smoke test, not yet a metrology claim. Clean-room PAIR
rectification, correspondence, and reprojection remain available as a separate
experimental path.

## Build

Install Rust and the normal Linux USB development package:

```sh
sudo apt install pkg-config libusb-1.0-0-dev
cargo build --release
```

## USB permissions

For a one-off test, run the utility with `sudo`. For regular use, install the
included udev rule instead:

```sh
sudo install -m 0644 udev/60-revopoint-pop3.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Then unplug and reconnect the scanner. The rule grants access to members of the
`plugdev` group. Log out and back in after adding yourself to that group.

## Read the current configuration

The read command reports only the SSID and whether a password is configured. It
does not print the password.

```sh
cargo run --release
```

Example:

```text
Current client SSID: "POP3Plus"
Current password configured: yes
```

## Provision Wi-Fi

Stop any application currently using the scanner, then run:

```sh
cargo run --release -- --write
```

The utility asks for the SSID, requests the password with terminal echo disabled,
and requires typing `WRITE` before touching the scanner. Client mode requires
both writing the client credentials and disabling the scanner's own access point.

After a successful write, disconnect the USB data cable and power-cycle the
scanner from a power adapter or power bank. When powered through a computer's USB
data connection, the scanner normally remains in USB mode.

## Smoke-test binocular infrared acquisition

With the scanner powered independently and connected to the same LAN, provide
its IPv4 address:

```sh
cargo run --release -- --smoke-pair 192.168.8.245 /tmp/pop3
```

The command captures exactly 1 MiB into a bounded reader, decompresses the
first complete frame, and saves raw left/right Y8 PGM images. It also downloads
the scanner's read-only left/right map parameters, writes rectified PGM images,
and writes **experimental** disparity and 16-bit millimeter-depth PGM files.
These are ordinary files that open in stock Linux image viewers; they are not
vendor formats. The
command enables the scanner's LED master and infrared projector immediately
before capture, waits 300 ms for illumination to settle, and turns both controls
off again on success or failure.

Example output:

```text
PAIR stream smoke passed: bytes=1048576, resolution=640x400, left=/tmp/pop3-left.pgm, right=/tmp/pop3-right.pgm, left_rectified=/tmp/pop3-left-rectified.pgm, right_rectified=/tmp/pop3-right-rectified.pgm, disparity=/tmp/pop3-disparity.pgm, depth_mm=/tmp/pop3-depth-mm.pgm, valid_pixels=... (...%), global_disparity_px=..., global_depth_mm=..., median_disparity_px=..., experimental_median_depth_mm=..., depth_mad_mm=..., depth_p10_p90_mm=.....
```

For the simpler scanner-computed depth slice:

```sh
cargo run --release -- --smoke-depth 192.168.8.245 /tmp/pop3-direct
```

Capture and camera-control commands now parse their scanner argument as an
explicit input mode. An IP address selects the working Wi-Fi transport; `usb`
selects the reserved direct-USB transport. Direct USB media acquisition is not
implemented yet and returns a specific error instead of silently falling back
to Wi-Fi:

```sh
cargo run --release -- --smoke-depth usb /tmp/pop3-direct
# Error: USB media acquisition is not implemented yet; use a scanner IP for Wi-Fi input
```

This boundary is intentional: decoding, calibration, registration, archiving,
and reconstruction remain independent of transport, while a future USB backend
will replace only acquisition and camera-control I/O.

This writes `/tmp/pop3-direct-depth-mm.pgm`, `-left.pgm`, and `-right.pgm`.
The depth PGM converts the scanner's 0.1 mm raw units to unsigned millimeters;
zero remains invalid. The command reports robust median, MAD, and p10–p90 depth
statistics so a measured planar target can be checked without trusting a
picture alone.

Depth capture no longer inherits an arbitrary exposure left behind by another
client. By default, `--smoke-pair`, `--smoke-depth`, `--smoke-rgbd`, and
`--capture-turntable` select the scanner's foreground-priority auto-exposure
mode before enabling the projector. The typed control API and CLI also expose
the manual and automatic controls recovered from the vendor SDK:

```sh
cargo run --release -- --depth-controls 192.168.8.245
cargo run --release -- --set-depth-auto-exposure 192.168.8.245 high-quality
cargo run --release -- --set-depth-exposure 192.168.8.245 5000
cargo run --release -- --smoke-rgbd 192.168.8.245 /tmp/pop3 \
  --depth-exposure 5000
```

Supported automatic modes are `off`, `fixed-frame-time`, `high-quality`, and
`foreground`. A manual exposure is checked against the range reported by that
scanner. The driver disables automatic exposure and, following the public SDK,
sets frame time to exposure plus 2,000 microseconds before applying the manual
value. Control writes require the firmware's `[ok]` acknowledgement. The
capture option can also be `--depth-auto-exposure MODE`; explicit settings are
applied as part of capture rather than relying on persistent device state.

The first turntable prerequisite—independent RGB acquisition—is also available:

```sh
cargo run --release -- --smoke-rgb 192.168.8.245 /tmp/pop3
cargo run --release -- --inspect-rgb-calibration 192.168.8.245
```

This uses the scanner's 1280×800 RGB stream and writes a validated ordinary
JPEG to `/tmp/pop3-rgb.jpg`. The driver separates the little-endian millisecond
timestamp that follows the JPEG rather than leaving non-JPEG bytes in the
output. The calibration command reads and validates the scanner's RGB
intrinsics, distortion, and left-depth-camera-to-RGB transform.

To exercise both live media endpoints together:

```sh
cargo run --release -- --smoke-rgbd 192.168.8.245 /tmp/pop3
```

This holds both media connections open until each parser has reached a complete
application-frame boundary, then writes `/tmp/pop3-depth-mm.pgm`,
`/tmp/pop3-rgb.jpg`, and `/tmp/pop3-colored.ply`. The images open in ordinary
Linux image viewers, while the binary little-endian colored point cloud opens
directly in CloudCompare. The command extracts both device-clock timestamps and
accepts the frames only when
RGB follows depth by no more than 50 ms. It collects eight candidates from each
stream and selects the closest valid pair. Color projection follows the public
SDK's depth-to-RGB transform convention. Registration accuracy still needs a
target with recognizable depth/color edges; the current wall proves the data
path and file interoperability, not pixel alignment.

The same command atomically publishes a replayable frame under
`/tmp/pop3-archive/frame-TIMESTAMP/`. Each frame contains the exact little-endian
Z16 plane, a viewable millimetre PGM, the original JPEG, the colored PLY, and a
versioned JSON manifest with both timestamps and all calibration needed for
registration. Rebuild the colored cloud with the scanner disconnected:

```sh
cargo run --release -- --replay-archive \
  /tmp/pop3-archive/frame-0025931490 /tmp/pop3-replayed.ply
```

Frame directories are renamed into place only after every artifact and the
manifest have been written. Unsafe path components, inconsistent timestamps,
unexpected filenames, malformed images, and attempts to overwrite a frame are
rejected.

For a stationary scanner and known-angle turntable, copy and edit
`examples/turntable-session.json`, replacing its example axis and center with
measured values in the left depth-camera frame. Generate the complete rotation
schedule once:

```sh
cargo run --release -- --write-turntable-schedule \
  examples/turntable-session.json /tmp/car-schedule
```

The generated files are ordinary per-frame metadata accepted by the capture
command. Rotate the object to the printed angle, then capture that index:

```sh
cargo run --release -- --capture-turntable \
  192.168.8.245 /tmp/object /tmp/car-schedule/frame-000000.json
```

The metadata records a safe session ID, frame index and expected count,
commanded and optional observed angles, an unambiguous direction viewed from
the axis tip, a unit rotation axis, and the turntable center in millimetres.
Turntable frame directories sort by zero-padded frame index, so an interrupted
session can resume at the first missing index without regenerating angles or
guessing from timestamps. Schedule generation refuses to overwrite an existing
directory.

Once every frame in the declared rotation exists, merge the session offline:

```sh
cargo run --release -- --merge-turntable \
  /tmp/object-archive car-rotation /tmp/car-rotation.ply
```

The merge reads calibration-backed points from each archive and applies the
inverse of the recorded object rotation around the declared axis and center.
It prefers an observed angle when present, sorts by the explicit frame index,
and rejects missing, duplicate, or inconsistent frames. The result is an
ordinary binary colored PLY for CloudCompare or MeshLab. This is deterministic
known-pose alignment, not ICP and not surface meshing; an inaccurate physical
axis or center will therefore remain visible rather than being hidden by an
optimizer.

The disparity image is currently a diagnostic, not qualified metric depth. It
uses a bounded 0–160 pixel search, a 15×15 SAD support window, a provisional 1%
best-versus-runner-up cost margin, and a one-pixel left/right consistency check.
It then requires eight locally consistent neighbors in a 5×5 window and applies
a local median. Occlusion-aware filling and physical scale validation remain
open.
The reported depth applies the scanner's read-only Q reprojection matrix to the
per-pixel disparity, including the calibration-to-stream resolution scale. The
16-bit PGM stores millimeters directly with zero for invalid pixels. It is
labelled experimental until the per-pixel result and a measured target pass the
hardware qualification matrix.

The global values come from an independent normalized whole-image horizontal
SAD sweep. They are diagnostic only: disagreement with the filtered per-pixel
distribution helps distinguish calibration/sign errors from local correspondence
ambiguity.

Offline network-boundary tests use loopback fixture servers and require no
scanner:

```sh
cargo test --test http_stream --test depth_capture --test camera_control
```

The repository retains separately tested Z16 decoding, calibration, ROS
message-mapping, and RViz configuration work. Direct Z16Y8Y8 acquisition is now
wired to the bounded smoke command; ROS remains wired to the experimental
host-reconstructed PAIR path pending a deliberate output-selection interface.

The independently established envelope is documented in
[`docs/depth-wire-observations.md`](docs/depth-wire-observations.md).

## Experimental ROS 2 / RViz path

This path deliberately retains the `experimental` label until measured-target
qualification. On Ubuntu 24.04 with ROS 2 Jazzy:

```sh
source /opt/ros/jazzy/setup.bash
cargo run --release --features ros2 -- --ros2-depth 192.168.8.245
```

The bounded command publishes 20 scanner-computed `32FC1` frames and matching
camera information on `/depth/image_rect` and `/depth/camera_info`. In separate
terminals, stock ROS tooling can derive the organized cloud and display both:

```sh
source /opt/ros/jazzy/setup.bash
ros2 run depth_image_proc point_cloud_xyz_node --ros-args \
  -r image_rect:=/depth/image_rect -r camera_info:=/depth/camera_info
ros2 run tf2_ros static_transform_publisher --frame-id world \
  --child-frame-id pop3_depth_optical_frame
rviz2 -d config/pop3-depth.rviz
```

A real-hardware direct-depth run delivered all 20 frames, a ROS subscriber
observed image width 640, and stock `depth_image_proc` emitted a 640-wide
`PointCloud2`. RViz2 launched with the supplied configuration and subscribed to
the live cloud; Wayland denied automated screenshot capture, so the direct-depth
visual geometry cell remains unaccepted. This verifies the automated plumbing,
not metrology or visual plane accuracy.

## Important limitations

- Only WPA2-PSK networks are currently generated.
- SSIDs are limited to 32 bytes and passwords to 8–63 bytes.
- WPA Enterprise, captive portals, and WPA3-only networks are not supported.
- The scanner briefly loses its kernel UVC driver while the command runs; the
  utility reattaches it before exiting.
- Only `2207:110c` is accepted. Other Revopoint products may use a related
  protocol, but they are deliberately not targeted without hardware testing.
- Live stereo depth reconstruction is experimental and not yet physically
  qualified. Current LAN acquisition yields the scanner's two Y8 infrared
  images plus diagnostic rectification, disparity, and median-depth outputs.

## How it works

The scanner exposes a vendor UVC extension unit on its USB control interface.
The utility uses libusb control transfers to access the scanner's ordinary
`wpa_supplicant` client configuration. File reads and writes are transferred in
56-byte payload blocks. No kernel module or custom driver is required.

The implementation is intentionally narrow: one known device, one configuration
file, and one filesystem-sync command.

## Recovery

Before writing, the utility displays the existing SSID and access-point state.
After writing, it verifies both configuration files before syncing. If
verification fails, do not power-cycle the scanner; rerun the utility or restore
the previous network details.

## License

MIT. See [LICENSE](LICENSE).
