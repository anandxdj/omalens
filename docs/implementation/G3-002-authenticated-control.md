# G3-002 — Authenticated restart control

Status: protocol, maintained keepalive channel, authenticated remote revocation, and both endpoints implemented; automated negative tests and builds pass; real-phone restart, discovery spoof, identity-change, and revocation trials remain.

Requirements: R03, R04, R05, R17. Gate: partial G3 only. Authentication establishes a control identity after pairing and deliberately cannot authorize camera capture.

## Selected protocol

The transport is TLS 1.3 from the maintained rustls/Android providers. The Android client accepts only the exact SHA-256 certificate fingerprint saved during the approved pairing ceremony. The desktop then challenges the phone to prove possession of the P-256 identity private key whose SPKI public key is in the mode-0600 trust record. Neither an mDNS result nor a display name grants trust.

The desktop sends one strict JSON `control_hello`, bounded by the 64 KiB control-message limit:

```json
{"type":"control_hello","min_v":1,"max_v":1,"features":0,"required":0,"sid":"<16 random bytes, base64url>","challenge":"<32 random bytes, base64url>","cert":"<TLS leaf SHA-256, base64url>"}
```

The phone rejects unknown fields, an incompatible version range, unsupported required features, malformed lengths, or a `cert` value different from the certificate it already pinned at TLS. It generates a new 32-byte nonce and signs the canonical transcript below with `SHA256withECDSA` using its Android Keystore identity:

```json
{"type":"control_proof","v":1,"features":0,"sid":"…","key":"<phone SPKI SHA-256>","nonce":"<32 random bytes>","sig":"<DER ECDSA signature>"}
```

The canonical transcript begins with the ASCII/UTF-8 bytes `OMACAM-CONTROL-PROOF-V1` followed by one NUL byte. Each subsequent field is encoded as a four-byte unsigned big-endian length followed by its bytes, in this exact order:

1. selected version as one byte;
2. selected feature mask as four unsigned big-endian bytes;
3. base64url session ID as UTF-8;
4. base64url challenge as UTF-8;
5. base64url desktop certificate digest as UTF-8;
6. raw 32-byte phone SPKI SHA-256 digest;
7. raw 32-byte phone nonce.

The desktop compares the claimed key digest to its stored peer, parses the stored SPKI as P-256, verifies the signature, and atomically consumes the challenge. A successful response is exactly:

```json
{"type":"authenticated","version":1,"capture_authorized":false}
```

The Android client treats `capture_authorized:true` as a protocol/security failure. Trust plus reconnect therefore leaves the independent capture state idle.

After authentication, the active app sends a bounded `{"type":"ping"}` every two seconds and requires `{"type":"pong","capture_authorized":false}`. `disconnect` ends only the connection. `forget_peer` is accepted only inside this authenticated TLS channel; the desktop deletes its trust record before attempting the best-effort `forgotten` acknowledgement and then stops advertising. Unknown fields or commands, missing keepalives, oversized messages, and stalled reads close the connection. This channel is control liveness only; no capture lease exists until a later explicit Start approval.

## Discovery and endpoint ownership

While explicitly running the control listener, the desktop advertises `_omacam._tcp.local.` only for the concrete address and port passed to `--listen`. TXT keys are `v=1`, `role=desktop-control`, and `cert=<saved desktop certificate digest>`. This record is an untrusted hint: the phone filters it by the saved digest and still requires exact TLS certificate pinning and the mutual proof above. OmaCam does not scan subnets, add routes, open a firewall, change interfaces, toggle tethering, or change USB functions.

The phone persists the approved desktop name and certificate digest, but no endpoint history, session material, QR token, or capture authorization. After restart it shows the trusted desktop and starts NSD only after **Find trusted laptop** is tapped. Android `CHANGE_WIFI_MULTICAST_STATE` is declared as a normal permission for local multicast discovery; no microphone permission is present.

## Revocation behavior

**Forget laptop** first clears the phone's saved desktop trust, sends `forget_peer` when an authenticated channel is available, and closes the connection. The desktop revokes locally before acknowledging the command. The phone retains its own Keystore identity so a future explicit pairing can consistently identify the same phone. `pair forget` removes only the desktop's trusted-phone record and retains the desktop identity. The running daemon re-reads trust before every keepalive response, so a local deletion tears down an active channel within the two-second phone cadence. Either side must be re-paired after its local record is removed. A future local D-Bus service will replace the development CLI but must preserve revoke-first ordering.

## Automated evidence

- Fresh trusted-key proof succeeds.
- Reusing a proof is rejected.
- Moving a proof to a different fresh challenge is rejected.
- A different/replaced phone key is rejected.
- An expired challenge is rejected.
- An unsupported version is rejected.
- A malformed signature is rejected.
- Pairing failures are capped at five per source and twenty globally for one invitation.
- `cargo test --workspace` and strict workspace Clippy pass after the implementation.
- Android debug assembly, lint, and unit-test tasks pass; there are currently no JVM unit-test sources.

## Required physical checks before closing G3

1. Reinstall the new debug APK without clearing data. Confirm it opens on the saved trusted-desktop screen rather than the scanner.
2. Start `control serve` on a concrete mutually reachable private address. Tap **Find trusted laptop** and confirm both screens say authenticated while neither requests camera permission nor starts capture.
3. Restart the desktop command and Android activity and repeat, proving stable identities with fresh challenges.
4. Advertise a forged service using the same TXT digest from a different TLS certificate; the phone must reject it at TLS.
5. Point the server at a newly generated desktop identity while retaining phone trust; the phone must report an identity change.
6. Replace the phone identity while retaining desktop trust; the desktop must reject it.
7. Forget on the phone, restart it, and confirm it returns to pairing. Separately forget on the desktop and confirm control refuses to start until pairing.
8. Keep the authenticated channel open, forget on the phone, and confirm the desktop deletes trust and stops advertising before acknowledging. Re-pair, run desktop-side Forget during an open channel, and confirm the next keepalive tears it down without capture.

Do not claim G3 complete from automated proof alone. Discovery reachability still depends on the existing LAN and firewall policy, and OmaCam must diagnose failure without changing either.
