# G9-001 — Browser-first, zero-install capture architecture

Status: in progress. Created 2026-09-11.

## Goal

Remove the installed Android companion from the critical path. A user points any
phone camera app (Google Lens, the stock camera QR reader, any scanner) at a QR
code shown by the Omarchy panel, taps the notification, and the phone becomes a
webcam. No application install, no account, no internet connection, no pairing
ceremony to memorise.

## Why this is possible

Three facts, verified 2026-09-11, make the flow legal in a current browser.

1. **A self-signed HTTPS origin is a secure context.** The Secure Contexts
   algorithm short-circuits on scheme: "If origin's scheme is either `https` or
   `wss`, return `Potentially Trustworthy`." There is no certificate-validation
   step anywhere in the algorithm. After the user taps through Chrome's
   interstitial once, `navigator.mediaDevices.getUserMedia` is available.

2. **Chrome's Local Network Access restrictions do not apply.** LNA shipped in
   Chrome 141 for desktop and Android, but it gates a *public* site reaching a
   local address, and it "applies to subresource requests, fetch() requests, and
   navigating subframes". Here the page is served *by* the desktop from its own
   LAN address, so every request is same-origin local-to-local. Nothing is
   gated. WebRTC is not covered at all yet — Chrome states it "plan[s] to send a
   separate Intent-to-Ship for applying LNA to WebSockets, WebTransport, and
   WebRTC connections."

3. **WebRTC gives us the transport for free.** The browser already contains a
   hardware H.264 encoder, congestion control, a jitter buffer, NACK
   retransmission and PLI keyframe requests. The desktop side is `str0m`, a
   sans-I/O Rust WebRTC implementation that builds with a pure-Rust crypto
   backend and no OpenSSL.

Consequence: the desktop needs no public domain, no CA-issued certificate, no
rendezvous server and no cloud account, and the phone needs no software.

## Flow

```text
Omarchy panel: "Use phone as camera"
  -> daemon mints a session token, renders a QR for
     https://<lan-ip>:<port>/#<token>
  -> phone camera app reads QR, opens Chrome
  -> Chrome interstitial: "Advanced" -> "Proceed" (once per certificate)
  -> page loads, shows the desktop name and a Connect button
  -> getUserMedia() -> Chrome's own camera permission prompt
  -> page shows 6 digits; panel shows the same 6 digits; user confirms on the
     desktop
  -> RTCPeerConnection offer POSTed to the daemon (same origin)
  -> str0m answers, ICE completes on the LAN, DTLS-SRTP established
  -> H.264 RTP -> depayloaded frames -> omacam-output -> virtual camera
```

Two taps of friction: the certificate interstitial and the camera permission.
The certificate exception is remembered per origin, so a returning user sees
only the camera prompt.

## Security model

The self-signed certificate is not trusted by a CA, so the page's authenticity
rests on:

- **The QR token.** A 256-bit random token in the URL fragment. The fragment is
  never sent in an HTTP request, so it does not appear in logs. The page reads
  it and presents it to `/api/session/claim`. Single use, 120-second lifetime,
  compared in constant time.
- **Certificate pinning through the QR.** The QR payload also carries the
  SHA-256 digest of the desktop certificate. The page verifies that the
  certificate it was served matches the digest in its own fragment, which
  detects an attacker who answered the LAN address first.
- **Short authentication string.** Six digits derived from the certificate
  digest and the session token, shown on both the phone and the desktop panel.
  The user confirms on the desktop. This is the same ceremony the companion app
  uses and it is what actually authenticates the pairing.
- **Explicit desktop approval.** Capture never starts from the phone alone.
  The panel must approve, exactly as today.

The existing safety boundary is unchanged: no networking, firewall, USB, ADB or
unrelated-camera state is modified.

## Components

| Component | Change |
| --- | --- |
| `omacam-core` | Add `webrtc_session`: token minting, SAS derivation, state machine. Sans-I/O, fully unit-testable. |
| `omacam-web` (new crate) | HTTPS listener, static asset serving, JSON API, `str0m` event loop, RTP-to-Annex-B bridge. |
| `webapp/` (new) | Single self-contained HTML file. No build step, no dependencies, no network fetches. |
| `omacam-daemon` | Serve the web provider alongside the existing control server; expose QR payload and SAS over D-Bus. |
| `omacam-output` | Unchanged interface; receives the same media records. |
| `omarchy-plugin` | Render the QR, show the SAS, add the approve action. |

The Android companion remains supported and unmodified. The browser provider is
promoted ahead of it in the provider order.

## Open items

- PipeWire camera node as an output alongside v4l2loopback (tracked separately).
- Bitrate adaptation is delegated to the browser's own congestion controller;
  confirm the applied bitrate is observable for R12 evidence.
