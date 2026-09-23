import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { buildCameraCapabilities, BrowserApi, offerFormatFromApplied, revealRole, supportedFormats, takeBearer } from "../shared-control.js";
import { PhoneSession, syncLocalPreview } from "../phone.js";

const cameraCapabilities = {
  width: { min: 320, max: 1920 },
  height: { min: 240, max: 1080 },
  frameRate: { min: 15, max: 60 },
  zoom: { min: 1, max: 4, step: 0.1 },
  exposureCompensation: { min: -2, max: 2, step: 0.5 },
  torch: true,
  focusMode: ["continuous"],
};

class FakeTarget {
  constructor() { this.listeners = new Map(); }
  addEventListener(type, handler) {
    const handlers = this.listeners.get(type) || new Set();
    handlers.add(handler);
    this.listeners.set(type, handlers);
  }
  removeEventListener(type, handler) { this.listeners.get(type)?.delete(handler); }
  emit(type) { for (const handler of this.listeners.get(type) || []) handler({ type, target: this }); }
}

class FakeTrack extends FakeTarget {
  constructor(deviceId = "rear-1") {
    super();
    this.kind = "video";
    this.deviceId = deviceId;
    this.stopped = false;
    this.settings = { deviceId, width: 1280, height: 720, frameRate: 30, zoom: 1, exposureCompensation: 0, torch: false };
  }
  getCapabilities() { return structuredClone(cameraCapabilities); }
  getSettings() { return { ...this.settings }; }
  getConstraints() { return {}; }
  async applyConstraints(constraints) {
    if (constraints.width?.exact) this.settings.width = constraints.width.exact;
    if (constraints.height?.exact) this.settings.height = constraints.height.exact;
    if (constraints.frameRate?.exact) this.settings.frameRate = constraints.frameRate.exact;
    const advanced = constraints.advanced?.[0] || {};
    if (typeof advanced.zoom === "number") this.settings.zoom = advanced.zoom;
    if (typeof advanced.exposureCompensation === "number") this.settings.exposureCompensation = advanced.exposureCompensation;
    if (typeof advanced.torch === "boolean") this.settings.torch = advanced.torch;
  }
  stop() { this.stopped = true; }
}

class FakeStream {
  constructor(track) { this.track = track; }
  getVideoTracks() { return [this.track]; }
  getTracks() { return [this.track]; }
}

class FakePeer extends FakeTarget {
  constructor() {
    super();
    this.iceGatheringState = "complete";
    this.connectionState = "new";
    this.closed = false;
    this.localDescription = null;
    this.sender = { track: null, replaceTrack: async (track) => { this.sender.track = track; } };
  }
  addTrack(track) { this.sender.track = track; return this.sender; }
  getTransceivers() { return [{ sender: this.sender, setCodecPreferences() {} }]; }
  async createOffer() { return { type: "offer", sdp: "v=0\r\n" }; }
  async setLocalDescription(description) { this.localDescription = description; }
  async setRemoteDescription(description) { this.remoteDescription = description; }
  close() { this.closed = true; this.connectionState = "closed"; }
}

function fixture({ getUserMedia, offer, heartbeat, onChange } = {}) {
  const track = new FakeTrack();
  const stream = new FakeStream(track);
  const peer = new FakePeer();
  const stopCalls = [];
  const intervalClears = [];
  let intervalId = 0;
  const lock = Object.assign(new FakeTarget(), {
    released: false,
    async release() { this.released = true; this.emit("release"); },
  });
  const doc = Object.assign(new FakeTarget(), { visibilityState: "visible" });
  const api = {
    offer: offer || (async () => ({ sessionId: "session-1", type: "answer", sdp: "v=0\r\n" })),
    heartbeat: heartbeat || (async () => ({ status: "live" })),
    stop: async (body) => { stopCalls.push(body); return { status: "stopped" }; },
  };
  const mediaDevices = {
    getUserMedia: getUserMedia || (async () => stream),
    enumerateDevices: async () => [{ kind: "videoinput", deviceId: "rear-1", label: "Back camera" }],
  };
  const session = new PhoneSession(api, {
    mediaDevices,
    RTCPeerConnection: class extends FakePeer { constructor() { super(); fixturePeer = this; } },
    navigator: { wakeLock: { request: async () => lock } },
    document: doc,
    setInterval: () => ++intervalId,
    clearInterval: (id) => intervalClears.push(id),
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    onChange,
  });
  let fixturePeer = peer;
  return { session, track, stream, peerRef: () => fixturePeer, stopCalls, lock, doc, intervalClears };
}

test("camera offer format uses the measured dimensions and rounded measured FPS", () => {
  assert.deepEqual(offerFormatFromApplied({ cameraId: "rear-1", width: 1280, height: 720, fps: 29.97 }), {
    width: 1280, height: 720, fps: 30,
  });
  assert.deepEqual(supportedFormats(cameraCapabilities).map(({ width, height, fps }) => ({ width, height, fps })), [
    { width: 1280, height: 720, fps: 30 },
    { width: 1920, height: 1080, fps: 30 },
  ]);
  assert.deepEqual(supportedFormats({ ...cameraCapabilities, width: { min: 320, max: 1280 } }).map((mode) => mode.width), [1280]);
});

test("capability payload advertises only enumerated devices and active-track modes", () => {
  const capabilities = buildCameraCapabilities([
    { kind: "videoinput", deviceId: "rear-1", label: "Back camera" },
    { kind: "videoinput", deviceId: "front-1", label: "Front camera" },
  ], "rear-1", cameraCapabilities, { frameRate: 30 });
  assert.equal(capabilities.cameras.length, 2);
  assert.equal(capabilities.cameras[0].modes.length, 2);
  assert.deepEqual(capabilities.cameras[1].modes, []);
  assert.equal(capabilities.torch, true);
  assert.deepEqual(capabilities.zoom, { min: 1, max: 4, step: 0.1 });
});

test("bearer is taken from the fragment and removed from the visible URL", () => {
  let cleaned = "";
  const fakeWindow = {
    location: { hash: "#token=desktop-secret", pathname: "/desktop", search: "" },
    history: { replaceState: (_state, _title, url) => { cleaned = url; } },
  };
  assert.equal(takeBearer("desktop", fakeWindow), "desktop-secret");
  assert.equal(cleaned, "/desktop");
});

test("each route reveals only its own page before token validation", () => {
  const phone = { hidden: true };
  const desktop = { hidden: true };
  const documentRef = { getElementById: (id) => id === "phone-app" ? phone : id === "desktop-app" ? desktop : null };
  revealRole(documentRef, "phone");
  assert.equal(phone.hidden, false);
  assert.equal(desktop.hidden, true);
  revealRole(documentRef, "desktop");
  assert.equal(phone.hidden, true);
  assert.equal(desktop.hidden, false);
});

test("API client sends bearer auth and JSON bodies on same-origin browser routes", async () => {
  let observed;
  const api = new BrowserApi("phone-secret", async (url, init) => {
    observed = { url, init, authorization: init.headers.get("Authorization") };
    return new Response(JSON.stringify({ status: "live" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });
  const body = { sessionId: "session-1", commandId: "command-1" };
  assert.deepEqual(await api.heartbeat(body), { status: "live" });
  assert.equal(observed.url, "/api/heartbeat");
  assert.equal(observed.authorization, "Bearer phone-secret");
  assert.deepEqual(JSON.parse(observed.init.body), body);
});

test("Stop during camera permission releases a late stream without starting signaling", async () => {
  let resolveCapture;
  let offerCalls = 0;
  const fx = fixture({
    getUserMedia: () => new Promise((resolve) => { resolveCapture = resolve; }),
    offer: async () => { offerCalls += 1; return { sessionId: "unexpected", type: "answer", sdp: "v=0" }; },
  });
  const start = fx.session.start();
  await new Promise((resolve) => setImmediate(resolve));
  await fx.session.stop("Stop pressed during permission prompt");
  resolveCapture(fx.stream);
  assert.equal(await start, false);
  assert.equal(fx.track.stopped, true);
  assert.equal(offerCalls, 0);
  assert.equal(fx.session.phase, "stopped");
});

test("late offer answer after Stop is stopped on the backend", async () => {
  let resolveOffer;
  const fx = fixture({
    offer: () => new Promise((resolve) => { resolveOffer = resolve; }),
  });
  const start = fx.session.start();
  while (!resolveOffer) await new Promise((resolve) => setImmediate(resolve));
  await fx.session.stop("Stop pressed during signaling");
  resolveOffer({ sessionId: "late-session", type: "answer", sdp: "v=0\r\n" });
  assert.equal(await start, false);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(fx.track.stopped, true);
  assert.deepEqual(fx.stopCalls, [{ sessionId: "late-session" }]);
  assert.equal(fx.peerRef().closed, true);
});

test("WebRTC failure stops capture, releases wake lock, and sends one server Stop", async () => {
  const fx = fixture();
  assert.equal(await fx.session.start(), true);
  assert.equal(fx.session.phase, "live");
  assert.ok(fx.session.capabilities.cameras.some((camera) => camera.id === "rear-1"));
  fx.peerRef().connectionState = "failed";
  fx.peerRef().emit("connectionstatechange");
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(fx.session.phase, "stopped");
  assert.equal(fx.track.stopped, true);
  assert.equal(fx.lock.released, true);
  assert.deepEqual(fx.stopCalls, [{ sessionId: "session-1" }]);
  assert.deepEqual(fx.intervalClears, [1]);
});

test("dim detaches the local preview, reveal restores it, and Stop still closes media", async () => {
  const preview = {
    srcObject: null,
    pauseCount: 0,
    playCount: 0,
    pause() { this.pauseCount += 1; },
    play() { this.playCount += 1; return Promise.resolve(); },
  };
  let fx;
  fx = fixture({ onChange: (state) => syncLocalPreview(preview, fx.stream, state.phase, state.applied) });
  assert.equal(await fx.session.start(), true);
  assert.equal(preview.srcObject, fx.stream);
  await fx.session.applyLocalControls({ screenDimmed: true });
  assert.equal(preview.srcObject, null);
  assert.ok(preview.pauseCount > 0);
  assert.equal(fx.track.stopped, false, "the sender track remains active while the preview is dimmed");
  await fx.session.reveal();
  assert.equal(preview.srcObject, fx.stream);
  await fx.session.stop();
  assert.equal(preview.srcObject, null);
  assert.equal(fx.track.stopped, true);
});

test("HTML keeps phone and desktop roles on external modules with keyboard accessible controls", async () => {
  const html = await readFile(new URL("../index.html", import.meta.url), "utf8");
  for (const id of ["phone-start", "phone-stop", "phone-reveal", "phone-dim-stop", "desktop-stop", "desktop-preview", "accent-picker"]) {
    assert.match(html, new RegExp(`id="${id}"`));
  }
  assert.match(html, /href="\/assets\/app\.css"/);
  assert.match(html, /src="\/assets\/phone\.js"/);
  assert.match(html, /src="\/assets\/desktop\.js"/);
  assert.doesNotMatch(html, /<script[^>]*>\s*[^<]/i);
});
