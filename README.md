# Revopoint POP 3 Linux interoperability

A Linux-native Rust project for interoperating with a Revopoint POP 3 scanner.
It currently provisions Wi-Fi client credentials over USB and performs bounded
depth-stream acquisition over the LAN. It avoids installing or running Revo
Scan, Windows, Wine, or a virtual machine.

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

The network acquisition path can configure the scanner's depth output, capture
an exact byte count from its chunked HTTP media stream, recover bounded frame
envelopes, decompress their QuickLZ payloads in Rust, and validate owned
640x400 little-endian Z16 planes. The device reports a depth unit of 0.1 mm.
The client also downloads and validates the scanner's depth projection matrix,
then scales its focal lengths and principal point to the selected stream size
using the same resolution transform documented by Revopoint's public SDK.
A live standard-application publisher is not yet implemented.

The in-progress optional ROS 2 adapter maps each Z16 plane to standard
`sensor_msgs/Image` `32FC1` meters so the scanner's 0.1 mm units are not
silently mislabeled as the `16UC1` millimeter convention. It produces matching
rectified `sensor_msgs/CameraInfo` metadata and uses sensor-data QoS. The exact
runtime message layouts are tested through Jazzy's installed type-support
libraries; the live publisher CLI is the next slice.

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

## Smoke-test depth acquisition

With the scanner powered independently and connected to the same LAN, provide
its IPv4 address:

```sh
cargo run --release -- --smoke-depth 192.168.8.245
```

The command captures exactly 1 MiB into a bounded reader, reports complete frame
envelopes and their compressed-to-raw sizes, retains short wire and decoded
prefixes for diagnostics, closes the scanner stream, and exits. It is intended
as a hardware diagnostic rather than a file format or visualization tool.

Example output:

```text
Depth stream smoke passed: bytes=1048576, resolution=640x400x2, stride=1280, millimeters_per_unit=0.1, calibration=1280x800, intrinsics=(fx=873.7233,fy=873.7233,cx=128.598,cy=201.2753), complete_frames=4, frame_receipts=[...]
```

Offline network-boundary tests use loopback fixture servers and require no
scanner:

```sh
cargo test --test http_stream --test depth_capture
```

The optional ROS adapter requires ROS 2 Jazzy to be sourced:

```sh
. /opt/ros/jazzy/setup.sh
cargo test --features ros2 --test ros2_dynamic_messages
```

The independently established envelope is documented in
[`docs/depth-wire-observations.md`](docs/depth-wire-observations.md).

## Important limitations

- Only WPA2-PSK networks are currently generated.
- SSIDs are limited to 32 bytes and passwords to 8–63 bytes.
- WPA Enterprise, captive portals, and WPA3-only networks are not supported.
- The scanner briefly loses its kernel UVC driver while the command runs; the
  utility reattaches it before exiting.
- Only `2207:110c` is accepted. Other Revopoint products may use a related
  protocol, but they are deliberately not targeted without hardware testing.

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
