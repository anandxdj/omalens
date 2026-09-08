# OmaCam — Product Requirements Document

**Status:** Architecture-ready PRD  
**Platform:** Omarchy / Arch Linux, Android-first  
**Product type:** Native Omarchy phone-camera bridge  
**Working name:** OmaCam

---

## 1. Product Vision

OmaCam makes an Android phone behave like a native webcam inside Omarchy.

The desired experience is:

```text
Click camera icon in Omarchy bar
        ↓
Known phone?
 ├─ Yes → Connect
 └─ No  → Show QR
                ↓
             Scan QR
                ↓
          Approve connection
                ↓
          Live camera preview
                ↓
        Choose phone camera
                ↓
        OmaCam Camera appears
        in Linux applications
```

The user should not have to understand:

- ADB
- IP addresses
- `/dev/video*`
- V4L2
- PipeWire
- codecs
- USB modes
- WebRTC
- camera IDs
- network interfaces

The product should feel like an Omarchy feature, not a collection of Linux scripts.

---

# 2. Core Product Promise

> **Turn your phone into an Omarchy webcam in seconds, without interfering with anything else the phone or laptop is already doing.**

OmaCam must prioritize:

1. ease of use;
2. low latency;
3. connection reliability;
4. compatibility;
5. preserving existing phone/laptop services;
6. native Omarchy integration;
7. adaptive support for different Android devices.

---

# 3. Hard Product Principles

## 3.1 Coexistence First

OmaCam is additive.

It must never intentionally disable or hijack:

```text
USB tethering
Wi-Fi hotspot
Wi-Fi internet
Ethernet
Bluetooth
USB file transfer
other laptop networking
other phone networking
```

to gain access to the camera.

Example:

```text
Phone
├── USB tethering ───────────────► Laptop internet
│
└── OmaCam camera ───────────────► OmaCam Camera
```

Both must continue simultaneously whenever the device/platform makes that possible.

Android's USB configuration supports composite configurations such as `rndis,adb`, demonstrating that USB tethering and ADB can coexist rather than being mutually exclusive. citeturn508585search9

OmaCam must therefore **adapt to the current USB state rather than changing it unnecessarily**.

---

## 3.2 One Phone, One Camera Device

OmaCam is intentionally **not** a multi-phone production system.

Only one phone is active at a time.

Linux applications see:

```text
OmaCam Camera
```

not:

```text
OmaCam OnePlus
OmaCam Pixel
OmaCam Samsung
...
```

The user can switch between cameras/lenses available on the connected phone.

This avoids substantial complexity with very little benefit for the expected Omarchy audience.

---

## 3.3 Stable Virtual Camera

Once OmaCam is active, applications should see one persistent camera:

```text
OmaCam Camera
```

Applications such as:

- Chrome
- Firefox
- Discord
- OBS
- Zoom
- Google Meet
- Electron applications

should not need to know whether the phone is using USB, Wi-Fi or another transport.

Transport switching or temporary disconnection should not destroy the virtual camera device.

---

# 4. Primary User Flow

## 4.1 Top Bar

OmaCam can be added or removed from the Omarchy bar like other widgets.

Omarchy already supports third-party `bar-widget`, `panel`, `service`, `menu`, and related plugin kinds through QML plugin manifests. citeturn441843search0turn441843search1

Typical bar state:

```text
󰄀
```

Hover:

```text
OmaCam
No camera connected
```

Connected:

```text
󰄀 •
```

The widget itself is optional.

The OmaCam background daemon continues working even when the widget is hidden.

---

# 5. Panel UX

Clicking the bar icon opens:

```text
╭──────────────────────────────────╮
│ OmaCam                           │
│                                  │
│       ┌──────────────────┐       │
│       │                  │       │
│       │   LIVE PREVIEW   │       │
│       │                  │       │
│       └──────────────────┘       │
│                                  │
│ Anand's Phone                    │
│ Connected • Wireless • 41 ms     │
│                                  │
│ Camera                           │
│ Main                         ▾    │
│                                  │
│ Quality                          │
│ Automatic                    ▾    │
│                                  │
│ Audio                            │
│ Phone microphone             ○   │
│ Laptop/default microphone    ●   │
│ No audio                     ○   │
│                                  │
│ Camera Controls                  │
│ Zoom            ━━━━●━━━━        │
│ Exposure        ━━━━━●━━━        │
│ Focus           Auto         ▾    │
│ Torch           ○                │
│                                  │
│ [ Fullscreen Preview ]           │
│                                  │
│ Advanced ▾                       │
│                                  │
│             [ Disconnect ]       │
╰──────────────────────────────────╯
```

No recording or screenshot functionality is included.

OmaCam remains focused on being a webcam bridge.

---

# 6. First-Time Pairing

If no trusted phone exists:

```text
╭──────────────────────────────╮
│ Connect your phone           │
│                              │
│         █▀▀▀▀▀▀█             │
│         █ QR  █              │
│         █▄▄▄▄▄▄█             │
│                              │
│ Scan with your phone         │
│                              │
│ Waiting for device…          │
╰──────────────────────────────╯
```

The QR contains a temporary secure pairing payload.

Conceptually:

```text
pairing session ID
laptop identity/public key
temporary authentication secret
available connection endpoint(s)
expiry
```

The QR must not contain a permanent reusable authentication secret.

---

# 7. Pairing Model

First pairing:

```text
Laptop
  ↓
Generate temporary pairing session
  ↓
Display QR
  ↓
Phone scans
  ↓
Secure handshake
  ↓
Device information shown
  ↓
User approval
  ↓
Trusted device stored
```

Future sessions:

```text
Known phone detected
        ↓
Per-device policy
        │
        ├─ Auto-connect
        ├─ Ask
        └─ Ignore
```

Trusted pairing should use device identities rather than reusable plaintext passwords.

---

# 8. Phone-Side Strategy

OmaCam uses a **hybrid capability model**.

The user should not need to know which implementation is active.

```text
QR scanned
    ↓
Capability detection
    │
    ├─ browser path sufficient
    │       ↓
    │    use browser/PWA
    │
    ├─ existing native capability available
    │       ↓
    │      use it
    │
    └─ advanced capabilities required
            ↓
      OmaCam companion
```

The objective is not "never install an app."

The objective is:

> **Never require an app unless the feature actually needs one.**

---

# 9. Why Browser-Only Is Not the Architecture

Browser camera APIs provide standardized capabilities such as:

- front/back facing selection;
- width;
- height;
- frame rate.

For example, `facingMode` commonly exposes `"user"` and `"environment"` camera directions. citeturn847728search8turn847728search11

But OmaCam needs more than that.

The desired product includes adaptive access to:

- ultrawide;
- main;
- telephoto;
- front camera;
- zoom;
- exposure;
- focus;
- torch;
- frame-rate combinations;
- physical-camera capabilities.

Android Camera2 exposes logical and physical camera relationships and allows applications to query physical camera IDs when OEM hardware exposes them. The exact set is device dependent. citeturn847728search1turn847728search5

Therefore:

### Browser/PWA

Used when it provides the required capability.

### OmaCam Android Companion

Used when advanced access, reliability or background behaviour requires native Camera2/MediaCodec access.

This remains invisible at the laptop UX level.

---

# 10. Phone Companion Philosophy

If installed, the Android component should remain extremely simple.

It is **not** supposed to become another complicated camera application.

Primary screen:

```text
╭─────────────────────────────╮
│ OmaCam                      │
│                             │
│ Connected to                │
│ Anand's Omarchy             │
│                             │
│        LIVE PREVIEW         │
│                             │
│ Main • 1080p60              │
│                             │
│ [ Black Screen ]            │
│                             │
│ Disconnect                  │
╰─────────────────────────────╯
```

Most controls remain on the laptop.

---

# 11. Black-Screen Camera Mode

During long webcam sessions the phone should remain sufficiently awake to maintain camera streaming.

Rather than forcing full Android lock/sleep behaviour, OmaCam provides:

```text
Black Screen Mode
```

The phone display renders an almost entirely black UI while the streaming process stays active.

Goals:

- minimize OLED power usage;
- reduce unnecessary screen brightness;
- avoid accidental UI interaction;
- preserve the camera connection;
- avoid Android killing the camera session.

The implementation may use wake locks or foreground-service behaviour where Android requires it.

DroidCam itself documents that some Android manufacturers restrict long-running background camera behaviour and that battery optimization can interrupt streaming, confirming that this needs explicit handling rather than assuming a browser/background session will remain alive indefinitely. citeturn747821search1

---

# 12. Camera Capability Model

OmaCam must never assume:

```text
Camera 0 = main
Camera 1 = selfie
Camera 2 = ultrawide
```

Instead, it discovers device capabilities.

Internally:

```text
logical camera
physical cameras
lens facing
focal lengths
sensor characteristics
supported sizes
supported FPS ranges
zoom range
focus modes
exposure capability
flash capability
manual sensor capability
```

Android explicitly warns developers not to assume a device has only one rear and one front camera. citeturn847728search1

---

# 13. Camera Selection UX

Default interface shows friendly names:

```text
Front
Main
Ultra-wide
Telephoto
```

where confidently identifiable.

If an OEM exposes unusual additional cameras:

```text
Advanced
└── Additional cameras
    ├── Rear camera 3
    └── Rear camera 5
```

Raw Android camera IDs must never be the normal UX.

---

# 14. Adaptive Controls

Controls are based entirely on actual device capabilities.

Example:

### Device A

```text
Main
Ultra-wide
Front
Zoom
Exposure
Torch
```

### Device B

```text
Main
Telephoto
Front
Zoom
Manual Focus
Exposure
ISO
Shutter
White Balance
Torch
```

The primary UI hides irrelevant controls.

Advanced capability information may show why a feature is unavailable.

Camera2 provides capability flags for features including manual exposure, sensitivity and lens control, while `CONTROL_ZOOM_RATIO` supports devices whose optical zoom spans different physical lenses. citeturn847728search0turn847728search1

---

# 15. Connection Architecture

OmaCam does **not** expose transport complexity to normal users.

Default:

```text
Connection
Automatic
```

Internally:

```text
                 Connection Manager
                        │
           ┌────────────┼────────────┐
           │            │            │
        Native       Wired        Wireless
         UVC         transport      transport
           │            │            │
           └────────────┴────────────┘
                        ↓
                   Media Engine
```

---

# 16. Native UVC Support

Android 14 QPR1 introduced Android's `DeviceAsWebcam` support for manufacturers that enable it. Compatible phones advertise themselves as UVC webcams to hosts such as Linux. citeturn508585search8

OmaCam should detect native UVC cameras where available.

However, it must **not automatically switch the phone's USB mode to UVC** if doing so interferes with tethering or another active function.

Native UVC is an available backend, not an excuse to violate the coexistence principle.

---

# 17. ADB / scrcpy Compatibility Backend

scrcpy already demonstrates a highly optimized Android camera path supporting:

- camera IDs;
- front/back camera selection;
- requested FPS;
- high-speed modes;
- torch;
- zoom;
- Linux V4L2 output. citeturn508585search2turn508585search5


Therefore the final OmaCam architecture should allow a scrcpy-derived/compatible backend where it is the most appropriate transport.

This is especially useful where:

```text
USB tethering
+
ADB
```

can coexist.

The product should wrap these details completely.

Users never run:

```bash
scrcpy ...
adb ...
```

themselves.

---

# 18. Wireless Transport

Wireless operation must support local peer-to-peer operation.

The normal experience:

```text
Phone
  │
local network
  │
Laptop
```

The video should remain local rather than being sent through an OmaCam cloud server.

Cloud infrastructure may assist with things such as version metadata, but must not be required for the camera's media path.

---

# 19. Transport Switching

OmaCam never changes connection method merely because another method is theoretically faster if that risks disturbing another service.

Example:

```text
Current:
USB tethering + wireless OmaCam

USB transport becomes detectable
```

OmaCam first evaluates:

```text
Can camera use USB
WITHOUT affecting tethering?
```

If yes:

```text
wireless → wired
```

can occur when automatic transport selection allows it.

If no:

```text
keep wireless
keep USB tethering
```

No interruption.

---

# 20. Transport Selection Policy

Conceptual scoring:

```text
candidate_score =
    compatibility
  + stability
  + latency
  + available bandwidth
  - service disruption risk
  - packet loss
  - thermal penalty
```

But:

```text
service disruption risk > 0
```

can disqualify a candidate entirely.

### Priority

```text
Do not break anything
        ↓
Maintain camera
        ↓
Stability
        ↓
Latency
        ↓
Image quality
```

---

# 21. Automatic Quality Mode

Default:

```text
Quality: Automatic
```

OmaCam continually observes:

```text
available bandwidth
packet loss
jitter
RTT
decode queue
encoder health
thermal state
frame drops
```

Possible adaptation:

```text
1080p60 @ 16 Mbps
        ↓ network weakens
1080p60 @ 11 Mbps
        ↓
1080p30 @ 8 Mbps
        ↓
720p30 @ 5 Mbps
```

The goal is to preserve a smooth camera rather than freeze repeatedly.

---

# 22. Manual Quality Overrides

Advanced users can override automatic behaviour.

Example:

```text
Mode
Automatic
Low Latency
Balanced
Maximum Quality
Custom
```

Custom may expose:

```text
Resolution
FPS
Bitrate
Codec
buffer strategy
```

These belong under Advanced, not the primary interface.

---

# 23. Performance Goals

These are engineering targets, not guaranteed marketing claims.

### Wired

Target end-to-end camera latency:

```text
Typical target: < 60 ms
Stretch target: < 40 ms
```

### Strong wireless network

```text
Typical target: < 100 ms
Preferred:      < 70 ms
```

### Startup

Known device available:

```text
click → usable preview
target: ≤ 2 seconds
```

### Recovery

Short network interruption:

```text
stream recovery target: ≤ 3 seconds
```

without removing `OmaCam Camera`.

---

# 24. Linux Media Architecture

```text
Phone
  │
encoded camera stream
  ↓
OmaCam Daemon
  │
  ├── transport
  ├── decoder
  ├── timing
  ├── camera state
  ├── audio state
  └── recovery
         │
         ↓
    Media Pipeline
       │      │
       │      └─────────► preview
       │
       └────────────────► virtual camera
```

---

# 25. Virtual Camera Output

Primary compatibility layer:

```text
v4l2loopback
```

Device label:

```text
OmaCam Camera
```

v4l2loopback supports custom card labels and an `exclusive_caps` mode specifically useful for applications such as Chrome/WebRTC that otherwise may have trouble recognizing loopback devices. citeturn508585search0

Installation should configure this automatically.

Users should never need to run `modprobe` manually during normal setup.

---

# 26. PipeWire

OmaCam should also integrate properly with PipeWire.

Modern Linux camera portals provide applications access to camera nodes through PipeWire, and camera nodes are identified using:

```text
media.class = Video/Source
media.role  = Camera
``` citeturn508585search1turn508585search3


Architecture should therefore be compatible with both:

```text
V4L2 compatibility
+
PipeWire-native applications
```

rather than assuming `/dev/videoX` is the entire future of Linux camera access.

---

# 27. Preview Pipeline

The Omarchy panel displays the same source that feeds the virtual camera.

It must not start a second independent phone stream merely to produce the preview.

Preferred:

```text
incoming stream
      ↓
decode once
      ↓
     tee
   ↙     ↘
preview  virtual camera
```

This reduces:

- phone encoder load;
- network bandwidth;
- CPU/GPU usage;
- heat;
- duplicated connections.

---

# 28. Phone Microphone

Phone audio is optional.

Primary interface:

```text
Audio Source

○ Phone microphone
● Laptop/default microphone
○ No audio
```

Selecting laptop microphone simply means OmaCam provides video only.

Selecting phone microphone creates an OmaCam audio source through PipeWire.

Camera functionality must never depend on microphone functionality.

---

# 29. A/V Synchronization

When phone microphone is enabled:

```text
phone camera timestamps
+
phone microphone timestamps
+
transport timing
+
laptop presentation clock
```

must be synchronized.

The architecture should preserve timestamps from capture through transport rather than regenerating arbitrary presentation timestamps on the laptop.

Clock offset and drift correction must be handled for long sessions.

---

# 30. Connection Failure Behaviour

Short failure:

```text
connection lost
      ↓
virtual camera remains alive
      ↓
automatic reconnect
      ↓
stream resumes
```

The Omarchy panel displays:

```text
Reconnecting…
```

Applications must not suddenly lose their camera device.

For long failures, the virtual camera may output a neutral black/reconnecting frame while keeping the device available.

---

# 31. Device Detection

OmaCam should detect:

```text
USB phone plugged in
known device appearing on network
trusted device becoming available
phone disappearing
USB mode changing
network interface changing
```

Detection must be event-driven where possible rather than aggressive constant polling.

Linux components may involve:

```text
udev
NetworkManager events
mDNS/service discovery
ADB device notifications
```

depending on transport.

---

# 32. Per-Device Behaviour

Trusted device settings:

```text
Device
Anand's Phone

Connection behavior
● Ask
○ Auto-connect
○ Ignore

Preferred quality
Automatic

Prefer wired when non-disruptive
On
```

These preferences survive reboots.

---

# 33. Simple vs Advanced UI

Default:

```text
Connected • Wireless • 43 ms
Main Camera
1080p60
Automatic
```

Advanced:

```text
Transport        Wi-Fi
Interface        wlan0
Codec            H.265
Resolution       1920 × 1080
FPS              60
Bitrate          13.8 Mbps
RTT              7 ms
Jitter           3.4 ms
Dropped Frames   0.08%
Decoder           Hardware
Virtual Camera    Ready
```

Advanced diagnostics are for troubleshooting and enthusiasts.

They must not clutter the primary experience.

---

# 34. Omarchy Architecture

Omarchy runs its desktop as a single long-lived `omarchy-shell` Quickshell process, and third-party plugins execute inside it. citeturn441843search0turn441843search1

Therefore:

```text
DO NOT:

omarchy-shell
    └── entire media engine
```

Instead:

```text
omarchy-shell
    │
    └── OmaCam QML
             │
             │ IPC
             ↓
       omacam-daemon
```

A decoder, network problem or USB bug must never crash the desktop shell.

---

# 35. OmaCam Components

```text
omacam/
│
├── omarchy-plugin/
│   ├── manifest.json
│   ├── BarWidget.qml
│   ├── Panel.qml
│   ├── onboarding/
│   └── components/
│
├── daemon/
│   ├── discovery/
│   ├── pairing/
│   ├── transport/
│   ├── camera/
│   ├── media/
│   ├── audio/
│   ├── v4l2/
│   ├── pipewire/
│   ├── recovery/
│   ├── diagnostics/
│   └── ipc/
│
├── android/
│   ├── pairing/
│   ├── camera/
│   ├── encoder/
│   ├── audio/
│   ├── black-screen/
│   ├── discovery/
│   └── transport/
│
├── web/
│   └── zero-install pairing/client
│
├── protocol/
│
├── packaging/
│   ├── arch/
│   ├── systemd/
│   └── udev/
│
└── tests/
```

---

# 36. Recommended Technology Choices

| Area | Technology |
|---|---|
| Omarchy UI | QML / Quickshell |
| Linux daemon | Rust |
| Linux async | Tokio |
| Linux IPC | D-Bus |
| Android native component | Kotlin |
| Android advanced camera | Camera2 |
| Android encoder | MediaCodec |
| Zero-install phone path | Web/PWA |
| Wireless media | WebRTC or purpose-built low-latency transport after benchmarking |
| Media processing | GStreamer / native hardware decode |
| Modern Linux media | PipeWire |
| Broad webcam compatibility | V4L2 + v4l2loopback |
| Device events | udev |
| Service lifecycle | systemd user service |
| Local discovery | mDNS |
| Packaging | Arch PKGBUILD + Omarchy plugin |

The media transport should be selected after benchmark prototypes rather than locking QUIC/WebRTC merely because one sounds theoretically superior.

---

# 37. Daemon Lifecycle

Run:

```text
omacam-daemon
```

as a user-level service.

Conceptually:

```text
systemctl --user enable --now omacam.service
```

But installation performs this automatically.

Daemon idle state should consume negligible resources.

The camera/decoder stack starts only when necessary.

---

# 38. Installation

Desired user experience:

```text
Omarchy
Setup
→ Plugins
→ Add OmaCam
→ Install
```

Installer handles:

```text
OmaCam shell plugin
OmaCam daemon
systemd user service
required packages
v4l2loopback
module configuration
PipeWire integration
udev rules where necessary
desktop permissions
```

The user should not manually edit kernel-module settings.

Omarchy supports installing third-party plugin Git repositories directly through `omarchy plugin add`, validating their manifests before activation. citeturn441843search0

---

# 39. Uninstallation

Removing OmaCam should clean up:

```text
plugin
service
daemon
OmaCam-specific config
OmaCam-created virtual camera configuration
OmaCam-specific udev rules
```

Shared dependencies should not be aggressively removed if another application may use them.

Trusted phone identities should optionally be removable separately.

---

# 40. Onboarding

First install:

### Screen 1

```text
Use your phone as your Omarchy camera.

Fast.
Wireless or wired.
No IP addresses.
```

### Screen 2

```text
Pair your phone

[ QR ]
```

### Screen 3

```text
Phone connected ✓

[ live preview ]
```

### Screen 4

```text
Testing virtual camera…

OmaCam Camera ✓
```

### Done

```text
You're ready.

Use "OmaCam Camera" from
Meet, Discord, OBS and other apps.
```

---

# 41. Permission Philosophy

Request permissions only when required.

Example:

```text
Camera
→ requested when user starts camera

Microphone
→ requested only when phone microphone selected

Local network
→ requested when wireless transport requires it

Background operation
→ requested when required for reliable streaming
```

Do not request microphone permission merely because the application *might* use microphone functionality someday.

---

# 42. Privacy

By default:

```text
phone ↔ laptop
```

Media remains local.

OmaCam must clearly indicate active camera usage on:

- phone;
- Omarchy panel;
- bar indicator.

No silent camera activation after pairing.

Even with auto-connect enabled, pairing trust does not mean OmaCam may silently activate the camera without whatever Android permission/foreground-camera requirements apply.

---

# 43. Security

Requirements:

- encrypted control channel;
- encrypted wireless media;
- ephemeral QR pairing token;
- QR expiry;
- device identity keys;
- trusted-device storage;
- replay protection;
- no permanently reusable password encoded in QR;
- no unauthenticated LAN camera endpoint;
- explicit forget-device function.

---

# 44. Compatibility Backends

Architecture should support multiple internal camera providers.

```text
CameraProvider
│
├── NativeUvcProvider
├── ScrcpyProvider
├── OmaCamNativeProvider
└── WebProvider
```

All normalize into one interface:

```text
getCapabilities()
start()
stop()
setLens()
setZoom()
setExposure()
setFocus()
setTorch()
getStats()
```

This is critical.

The UI should not contain backend-specific logic.

---

# 45. Capability Normalization

Each provider returns something equivalent to:

```text
DeviceCapabilities

cameras[]
  id
  friendlyName
  facing
  focalLength
  resolutions[]
  fpsRanges[]

controls
  zoom
  autofocus
  manualFocus
  exposureCompensation
  torch
  whiteBalance
  iso
  shutter

audio
  microphoneAvailable
```

OmaCam UI renders based on this schema.

---

# 46. Provider Selection

Provider decision engine evaluates:

```text
device
Android version
native UVC availability
ADB availability
current USB configuration
tethering state
browser capabilities
OmaCam companion availability
network interfaces
requested controls
```

Example:

```text
Need:
Main camera
1080p60
zoom only

Browser can satisfy?
YES

→ web provider
```

Another:

```text
Need:
Telephoto
manual exposure
manual focus

Browser capability insufficient

OmaCam companion installed?
YES

→ native provider
```

The transition should feel invisible to the laptop user.

---

# 47. USB Tethering Scenario

This is a mandatory acceptance scenario.

Initial state:

```text
Phone
  │ USB
  ↓
Laptop

USB tethering = ACTIVE
Internet = working
```

User opens OmaCam.

Expected:

```text
Internet remains active
USB interface remains active
camera connects using a compatible backend
```

Unacceptable:

```text
OmaCam changes USB profile
→ tethering disconnects
```

This is a release-blocking failure.

---

# 48. Hotspot Scenario

Initial state:

```text
Phone provides hotspot
Laptop connected to hotspot
```

OmaCam should be able to use that same connection when technically viable.

Expected:

```text
Internet/hotspot continues
+
camera stream continues
```

OmaCam must not turn hotspot off to create its own network unless the user explicitly requests such behaviour.

---

# 49. Failure States

Examples:

### Phone unavailable

```text
Phone not found
Make sure OmaCam is open or the device is connected.
```

### Camera occupied

```text
Camera is being used by another app.
```

### Permission denied

```text
Camera permission is required.
[ Open phone instructions ]
```

### v4l2loopback unavailable

```text
OmaCam Camera could not be created.
[ Repair ]
```

### Unsupported lens

```text
That camera is unavailable on this device.
Switching back to Main.
```

### Network unstable

```text
Connection unstable
Quality adjusted automatically.
```

No raw stack traces in normal UX.

---

# 50. Repair System

One-click:

```text
Diagnostics
→ Repair OmaCam
```

Checks:

```text
daemon
PipeWire
v4l2loopback
kernel module compatibility
permissions
virtual camera
network discovery
ADB when applicable
phone connection
```

It should distinguish:

```text
Phone connected ✓
Camera stream ✓
Decoder ✓
PipeWire ✓
Virtual camera ✕
```

rather than merely returning:

```text
Connection failed.
```

---

# 51. Update Behaviour

OmaCam should follow normal Omarchy plugin update expectations where possible.

Daemon/protocol compatibility must be versioned.

Handshake includes:

```text
protocolVersion
clientVersion
daemonVersion
```

Unsupported mismatch:

```text
Phone component needs updating.

[ Update instructions ]
```

Old protocol versions should remain supported within a defined compatibility window.

---

# 52. Architecture Portability

OmaCam is:

> **Omarchy-first, not Omarchy-locked.**

The core daemon must not import or depend on Quickshell.

```text
             OmaCam Core
                  │
         desktop-neutral IPC
                  │
      ┌───────────┴───────────┐
      ↓                       ↓
Omarchy Plugin         Future Linux UI
```

Future environments could include:

```text
KDE
GNOME
other Wayland desktops
```

without rewriting:

- pairing;
- transport;
- camera;
- decoding;
- virtual-camera handling.

Omarchy remains the flagship experience.

---

# 53. Explicit Non-Goals

OmaCam is not:

- a video recorder;
- OBS replacement;
- livestreaming suite;
- multi-camera production switcher;
- phone file manager;
- phone screen mirroring application;
- remote desktop product;
- cloud camera service;
- surveillance system.

No multi-phone support in the initial product definition.

No snapshot/recording UI.

The product remains deliberately narrow:

> **phone → webcam.**

---

# 54. Success Criteria

A successful OmaCam user should be able to:

```text
install OmaCam
     ↓
click bar icon
     ↓
scan QR
     ↓
approve camera
     ↓
see preview
     ↓
select Main / Ultrawide / Telephoto / Front
     ↓
open Google Meet
     ↓
select "OmaCam Camera"
```

without opening a terminal.

---

# 55. Acceptance Requirements

OmaCam is product-ready only when all of the following are true.

### Pairing

- QR pairing works.
- trusted-device identity persists.
- QR token expires.
- forgotten devices can no longer reconnect.

### Camera

- front and rear camera detection works.
- additional exposed lenses are represented when available.
- unsupported controls are automatically hidden.
- camera changes do not require restarting the Linux application where avoidable.

### Virtual camera

- `OmaCam Camera` appears consistently.
- Chrome recognizes it.
- Firefox recognizes it.
- OBS recognizes it.
- Electron/WebRTC applications can use it.

### USB

- plugging/unplugging is detected.
- USB tethering is not interrupted.
- existing USB networking is not unnecessarily changed.

### Wireless

- QR pairing does not require manually entering an IP address.
- known devices can be rediscovered.
- brief network interruptions recover automatically.

### Omarchy

- widget can be added/removed.
- panel follows Omarchy visual conventions.
- daemon keeps running if widget is hidden.
- camera failures cannot crash `omarchy-shell`.

### Phone

- black-screen mode works while camera remains active.
- camera permissions are explicit.
- microphone permission is requested only when needed.

### Performance

- preview is usable for realtime calls.
- adaptive mode responds to deteriorating wireless conditions.
- CPU usage remains reasonable with hardware decode available.
- daemon is lightweight when idle.

---

# 56. Architecture Decision Summary

The final product architecture is:

```text
                         ANDROID PHONE
                              │
                ┌─────────────┼─────────────┐
                │             │             │
             Browser       Native       Existing
             / PWA        Companion     capability
                │             │             │
                └─────────────┼─────────────┘
                              │
                       Secure transport
                              │
                    USB / LAN / Wireless
                              │
                              ▼
                     ┌────────────────┐
                     │ omacam-daemon  │
                     │     Rust       │
                     └───────┬────────┘
                             │
                ┌────────────┼────────────┐
                │            │            │
              Decode       Audio       Diagnostics
                │            │
                └──────┬─────┘
                       │
                  media output
                 ┌─────┴──────┐
                 ▼            ▼
             PipeWire     V4L2loopback
                 │            │
                 └─────┬──────┘
                       ▼
                  OmaCam Camera
                       │
             Chrome / Meet / OBS /
              Discord / Firefox
                       ▲
                       │
                OmaCam QML panel
                       │
                    D-Bus IPC
                       │
                 omarchy-shell
```

---

# 57. Most Important Engineering Rule

Every architectural choice should be evaluated against this question:

> **Does this make OmaCam easier to use without disturbing something the user already has working?**

If an optimization saves 15 ms but disconnects USB tethering, reject it.

If a theoretically elegant transport requires the user to manually find an IP address, avoid it.

If supporting three simultaneous phones doubles the complexity for almost nobody, don't build it.

If advanced camera controls require the companion application, use the companion application rather than shipping unreliable browser hacks.

The result should be boringly simple for the user even if the implementation underneath is sophisticated.

---

# 58. Product Definition in One Sentence

> **OmaCam is an Omarchy-native, low-latency Android webcam bridge that securely pairs by QR, automatically chooses a non-disruptive wired or wireless connection, exposes the phone's supported cameras and controls through a native live-preview panel, and provides one persistent `OmaCam Camera` device to Linux applications without interfering with tethering, hotspot, networking, or other existing services.**