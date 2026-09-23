import {
  BrowserApi,
  buildCameraCapabilities,
  describeError,
  getNumericRange,
  initAccentControl,
  makeTrackConstraints,
  offerFormatFromApplied,
  readAppliedState,
  readCapabilities,
  readSettings,
  revealRole,
  supportedFormats,
  takeBearer,
  waitForIceGathering,
} from "./shared-control.js";

const HEARTBEAT_MS = 2_000;
const DISCONNECT_GRACE_MS = 2_500;

function once(target, eventName, listener) {
  if (typeof target?.addEventListener === "function") target.addEventListener(eventName, listener);
  else if (target) target[`on${eventName}`] = listener;
}

function stopStream(stream) {
  for (const track of stream?.getTracks?.() || []) {
    try { track.stop(); } catch { /* A stale or already stopped track is released. */ }
  }
}

function clampRange(range) {
  if (!range) return null;
  const min = Math.max(15, Math.ceil(range.min));
  const max = Math.min(60, Math.floor(range.max));
  return min <= max ? { min, max, step: range.step || 1 } : null;
}

export class PhoneSession {
  constructor(api, options = {}) {
    this.api = api;
    this.mediaDevices = options.mediaDevices ?? globalThis.navigator?.mediaDevices;
    this.RTCPeerConnection = options.RTCPeerConnection ?? globalThis.RTCPeerConnection;
    this.navigator = options.navigator ?? globalThis.navigator;
    this.document = options.document ?? globalThis.document;
    this.setInterval = options.setInterval ?? globalThis.setInterval;
    this.clearInterval = options.clearInterval ?? globalThis.clearInterval;
    this.setTimeout = options.setTimeout ?? globalThis.setTimeout;
    this.clearTimeout = options.clearTimeout ?? globalThis.clearTimeout;
    this.onChange = options.onChange ?? (() => {});
    this.onStatus = options.onStatus ?? (() => {});
    this.generation = 0;
    this.phase = "stopped";
    this.stream = null;
    this.track = null;
    this.peer = null;
    this.videoSender = null;
    this.sessionId = "";
    this.applied = null;
    this.capabilities = { cameras: [], torch: false, screenDimSupported: true };
    this.devices = [];
    this.verifiedModes = [];
    this.heartbeatTimer = null;
    this.disconnectTimer = null;
    this.heartbeatPromise = null;
    this.pendingCommandId = "";
    this.pendingCommandError = "";
    this.wakeLock = null;
    this.visibilityHandler = null;
    this.heartbeatFailures = 0;
    this.lastError = "";
  }

  get live() { return this.phase === "live"; }

  async prepare({ cameraId = "" } = {}) {
    if (this.phase === "prepared") return true;
    if (["preparing", "connecting", "live"].includes(this.phase)) return false;
    if (!this.mediaDevices?.getUserMedia || !this.RTCPeerConnection) {
      this.onStatus("error", "This browser does not support camera capture or WebRTC.");
      return false;
    }
    const generation = ++this.generation;
    this.phase = "preparing";
    this.sessionId = "";
    this.lastError = "";
    this.onStatus("starting", "Requesting camera permission…");
    this.onChange({ phase: this.phase, applied: null, capabilities: this.capabilities });

    try {
      const stream = await this.mediaDevices.getUserMedia({
        audio: false,
        video: cameraId
          ? { deviceId: { exact: cameraId }, width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } }
          : { facingMode: { ideal: "environment" }, width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } },
      });
      if (generation !== this.generation) {
        stopStream(stream);
        return false;
      }
      this.stream = stream;
      this.track = stream.getVideoTracks?.()[0] || stream.getTracks?.().find((item) => item.kind === "video");
      if (!this.track) throw new Error("The selected camera did not provide a video track.");

      this.devices = await this._enumerateDevices();
      if (generation !== this.generation) return false;
      this.verifiedModes = await this._selectStartupMode(this.track, generation);
      if (generation !== this.generation) return false;
      this.applied = readAppliedState(this.track, { cameraId: this._activeCameraId() });
      if (!this.applied) throw new Error("The camera did not report its actual size and frame rate.");
      this.capabilities = buildCameraCapabilities(this.devices, this.applied.cameraId, readCapabilities(this.track), readSettings(this.track), this.verifiedModes);
      this.phase = "prepared";
      this._watchTrack(this.track, generation);
      this.onStatus("ready", "Camera ready · choose quality before streaming");
      this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices });
      return true;
    } catch (error) {
      if (generation !== this.generation) return false;
      const message = describeError(error, "Camera setup failed.");
      this.lastError = message;
      await this._finishLocal(message, { notifyServer: true });
      this.onStatus("error", message);
      this.onChange({ phase: "stopped", applied: null, capabilities: this.capabilities, error: message });
      return false;
    }
  }

  async start({ cameraId = "" } = {}) {
    if (["preparing", "connecting", "live"].includes(this.phase)) return false;
    if (this.phase !== "prepared") {
      const prepared = await this.prepare({ cameraId });
      if (!prepared || this.phase !== "prepared") return false;
    }
    const generation = this.generation;
    this.phase = "connecting";
    this.onStatus("starting", "Starting local camera link…");
    this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices });
    try {
      const peer = new this.RTCPeerConnection({ iceServers: [] });
      this.peer = peer;
      this.videoSender = peer.addTrack(this.track, this.stream);
      this._configureVideoCodec(peer);
      this._watchPeer(peer, generation);

      const offer = await peer.createOffer();
      if (generation !== this.generation) return false;
      await peer.setLocalDescription(offer);
      await waitForIceGathering(peer);
      if (generation !== this.generation) return false;
      const localDescription = peer.localDescription || offer;
      const format = offerFormatFromApplied(this.applied);
      const answer = await this.api.offer({
        offer: { type: localDescription.type || "offer", sdp: localDescription.sdp },
        format,
        capabilities: this.capabilities,
        applied: this.applied,
      });

      if (!answer?.sessionId || !answer?.sdp) throw new Error("The local service returned an incomplete WebRTC answer.");
      this.sessionId = answer.sessionId;
      if (generation !== this.generation) {
        void this.api.stop({ sessionId: answer.sessionId }).catch(() => {});
        return false;
      }
      await peer.setRemoteDescription({ type: answer.type || "answer", sdp: answer.sdp });
      if (generation !== this.generation) {
        void this.api.stop({ sessionId: answer.sessionId }).catch(() => {});
        return false;
      }

      this.phase = "live";
      this._attachWakeLockLifecycle(generation);
      this.onStatus("live", "Camera streaming");
      this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices });
      await this._pulse(generation);
      if (generation === this.generation && this.phase === "live") {
        this.heartbeatTimer = this.setInterval(() => { void this._pulse(generation); }, HEARTBEAT_MS);
      }
      return generation === this.generation && this.phase === "live";
    } catch (error) {
      if (generation !== this.generation) return false;
      const message = describeError(error, "Camera setup failed.");
      this.lastError = message;
      await this._finishLocal(message, { notifyServer: true });
      this.onStatus("error", message);
      this.onChange({ phase: "stopped", applied: null, capabilities: this.capabilities, error: message });
      return false;
    }
  }

  async stop(reason = "Camera stopped") {
    if (this.phase === "stopped" && !this.stream && !this.peer) return;
    await this._finishLocal(reason, { notifyServer: true });
    this.onStatus("stopped", reason);
    this.onChange({ phase: "stopped", applied: null, capabilities: this.capabilities });
  }

  async reveal() {
    if (!["prepared", "live"].includes(this.phase) || !this.applied?.screenDimmed) return;
    this.applied = { ...this.applied, screenDimmed: false };
    this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices });
    if (this.live) void this._pulse(this.generation);
  }

  async applyLocalControls(controls = {}) {
    if (!["prepared", "live"].includes(this.phase) || !this.track) return false;
    try {
      await this._applyControls(controls, this.generation);
      this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices });
      if (this.live) void this._pulse(this.generation);
      return true;
    } catch (error) {
      const message = describeError(error, "The camera did not accept that setting.");
      this.onStatus("live", message);
      this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices, error: message });
      return false;
    }
  }

  async _enumerateDevices() {
    try {
      const devices = await this.mediaDevices.enumerateDevices();
      return Array.isArray(devices) ? devices.filter((device) => device.kind === "videoinput") : [];
    } catch { return []; }
  }

  _activeCameraId() {
    return readSettings(this.track).deviceId || this.track?.getConstraints?.().deviceId?.exact || "";
  }

  async _selectStartupMode(track, generation) {
    const original = readSettings(track);
    const choices = supportedFormats(readCapabilities(track));
    const verified = [];
    for (const mode of choices) {
      try {
        await track.applyConstraints({
          width: { exact: mode.width },
          height: { exact: mode.height },
          frameRate: { exact: mode.fps },
        });
        const actual = readSettings(track);
        if (actual.width === mode.width && actual.height === mode.height
            && Number.isFinite(actual.frameRate) && Math.round(actual.frameRate) === mode.fps) {
          verified.push({ width: mode.width, height: mode.height, fps: mode.fps });
        }
      } catch {
        // Range support suggests the preset; the driver's exact result remains authoritative.
      }
      if (generation !== this.generation) return;
    }
    const preferred = verified.find((mode) => mode.width === original.width && mode.height === original.height)
      || verified.find((mode) => mode.width === 1280)
      || verified[0];
    const restore = preferred || original;
    if (restore?.width && restore?.height && restore?.frameRate) {
      try {
        await track.applyConstraints({
          width: { exact: restore.width },
          height: { exact: restore.height },
          frameRate: { exact: preferred?.fps || restore.frameRate },
        });
      } catch { /* Keep the last actual settings if a driver refuses restoration. */ }
    }
    return verified;
  }

  _configureVideoCodec(peer) {
    const sender = globalThis.RTCRtpSender;
    const codecs = sender?.getCapabilities?.("video")?.codecs;
    if (!Array.isArray(codecs) || !codecs.length) return;
    const h264 = codecs.filter((codec) => codec.mimeType?.toLowerCase() === "video/h264");
    if (!h264.length) throw new Error("This browser does not offer H.264 video for the local camera link.");
    const rest = codecs.filter((codec) => !h264.includes(codec));
    const transceiver = peer.getTransceivers?.().find((item) => item.sender?.track?.kind === "video");
    try { transceiver?.setCodecPreferences?.([...h264, ...rest]); }
    catch { /* Negotiation will report a readable error if H.264 cannot be selected. */ }
  }

  _watchPeer(peer, generation) {
    const stateChanged = () => {
      if (generation !== this.generation) return;
      const state = peer.connectionState;
      if (state === "connected") {
        if (this.disconnectTimer) this.clearTimeout(this.disconnectTimer);
        this.disconnectTimer = null;
        if (this.phase === "live") this.onStatus("live", "Camera streaming");
      } else if (state === "failed" || state === "closed") {
        void this._connectionEnded(generation, state === "failed" ? "WebRTC connection failed" : "WebRTC connection closed");
      } else if (state === "disconnected" && !this.disconnectTimer) {
        this.disconnectTimer = this.setTimeout(() => {
          this.disconnectTimer = null;
          if (generation === this.generation && peer.connectionState === "disconnected") {
            void this._connectionEnded(generation, "Phone connection was lost");
          }
        }, DISCONNECT_GRACE_MS);
        this.onStatus("starting", "Reconnecting local camera link…");
      }
    };
    once(peer, "connectionstatechange", stateChanged);
    once(peer, "iceconnectionstatechange", stateChanged);
    stateChanged();
  }

  _watchTrack(track, generation) {
    once(track, "ended", () => {
      if (generation === this.generation && this.phase !== "stopped") {
        void this._connectionEnded(generation, "Camera track ended");
      }
    });
  }

  async _connectionEnded(generation, reason) {
    if (generation !== this.generation || this.phase === "stopped") return;
    await this._finishLocal(reason, { notifyServer: true });
    this.onStatus("error", reason);
    this.onChange({ phase: "stopped", applied: null, capabilities: this.capabilities, error: reason });
  }

  _attachWakeLockLifecycle(generation) {
    this.visibilityHandler = () => {
      if (generation !== this.generation || this.phase !== "live") return;
      if (this.document?.visibilityState === "visible") void this._requestWakeLock(generation);
      else void this._releaseWakeLock();
    };
    this.document?.addEventListener?.("visibilitychange", this.visibilityHandler);
    void this._requestWakeLock(generation);
  }

  async _requestWakeLock(generation) {
    if (generation !== this.generation || this.phase !== "live"
        || this.document?.visibilityState === "hidden" || !this.navigator?.wakeLock?.request) return;
    try {
      if (this.wakeLock && !this.wakeLock.released) return;
      const lock = await this.navigator.wakeLock.request("screen");
      if (generation !== this.generation || this.phase !== "live") {
        await lock.release?.();
        return;
      }
      this.wakeLock = lock;
      once(lock, "release", () => {
        if (this.wakeLock === lock) this.wakeLock = null;
        if (generation === this.generation && this.phase === "live"
            && this.document?.visibilityState === "visible") {
          void this._requestWakeLock(generation);
        }
      });
    } catch { /* Wake lock is optional and browser policy may deny it. */ }
  }

  async _releaseWakeLock() {
    const lock = this.wakeLock;
    this.wakeLock = null;
    if (lock && !lock.released) {
      try { await lock.release?.(); } catch { /* It may already have been released by the browser. */ }
    }
  }

  async _applyControls(controls, generation, { remote = false } = {}) {
    if (!remote && !["prepared", "live"].includes(this.phase)) throw new Error("Prepare the camera before changing capture settings.");
    if (remote && !this.live) throw new Error("Remote camera controls are available only while streaming.");
    if (this.live && ["width", "height", "fps"].some((key) => controls[key] != null)) {
      throw new Error("Resolution and frame rate are fixed for a live stream. Stop and restart to change them.");
    }
    if (generation !== this.generation) throw new DOMException("Session ended", "AbortError");

    if (controls.cameraId && controls.cameraId !== this._activeCameraId()) {
      await this._switchCamera(controls.cameraId, generation, controls);
    } else {
      const caps = readCapabilities(this.track);
      const requestedAdvanced = ["zoom", "exposure", "torch"].some((key) => controls[key] != null);
      if (controls.zoom != null && !getNumericRange(caps.zoom)) throw new Error("Zoom is not supported by this camera.");
      if (controls.exposure != null && !getNumericRange(caps.exposureCompensation)) throw new Error("Exposure control is not supported by this camera.");
      if (controls.torch != null && caps.torch !== true) throw new Error("Torch is not supported by this camera.");
      const trackConstraints = makeTrackConstraints(controls, caps);
      if (Object.keys(trackConstraints).length) await this.track.applyConstraints(trackConstraints);
      if (generation !== this.generation) throw new DOMException("Session ended", "AbortError");
      if (requestedAdvanced && !Object.keys(trackConstraints).length) throw new Error("The camera did not accept that setting.");
    }

    const fallback = {
      ...this.applied,
      cameraId: this._activeCameraId(),
      torch: controls.torch ?? this.applied?.torch,
      previewMirrored: controls.previewMirrored ?? this.applied?.previewMirrored,
      screenDimmed: controls.screenDimmed ?? this.applied?.screenDimmed,
      zoom: controls.zoom ?? this.applied?.zoom,
      exposure: controls.exposure ?? this.applied?.exposure,
    };
    const applied = readAppliedState(this.track, fallback);
    if (!applied) throw new Error("The camera stopped reporting its active capture settings.");
    if (controls.cameraId && applied.cameraId !== controls.cameraId) {
      throw new Error("The camera did not switch to the requested device.");
    }
    this.applied = applied;
    this.capabilities = buildCameraCapabilities(this.devices, applied.cameraId, readCapabilities(this.track), readSettings(this.track), this.verifiedModes);
    this._watchTrack(this.track, generation);
    this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices });
  }

  async _switchCamera(cameraId, generation, controls) {
    if (!this.capabilities.cameras.some((camera) => camera.id === cameraId)) {
      throw new Error("That camera is no longer available.");
    }
    const oldStream = this.stream;
    const current = this.applied;
    const stream = await this.mediaDevices.getUserMedia({
      audio: false,
      video: {
        deviceId: { exact: cameraId },
        width: { ideal: controls.width || current.width },
        height: { ideal: controls.height || current.height },
        frameRate: { ideal: controls.fps || current.fps },
      },
    });
    if (generation !== this.generation) {
      stopStream(stream);
      throw new DOMException("Session ended", "AbortError");
    }
    const nextTrack = stream.getVideoTracks?.()[0] || stream.getTracks?.().find((item) => item.kind === "video");
    if (!nextTrack) {
      stopStream(stream);
      throw new Error("The selected camera did not provide a video track.");
    }
    const nextCaps = readCapabilities(nextTrack);
    let verifiedModes = [];
    try {
      if (controls.width != null || controls.height != null || controls.fps != null
          || controls.zoom != null || controls.exposure != null || controls.torch != null) {
        const rangeCheck = (key, range) => controls[key] == null || (range && range.min <= controls[key] && controls[key] <= range.max);
        if (!rangeCheck("zoom", getNumericRange(nextCaps.zoom))) throw new Error("Zoom is not supported on the selected camera.");
        if (!rangeCheck("exposure", getNumericRange(nextCaps.exposureCompensation))) throw new Error("Exposure is not supported on the selected camera.");
        if (controls.torch != null && nextCaps.torch !== true) throw new Error("Torch is not supported on the selected camera.");
        const constraints = makeTrackConstraints(controls, nextCaps);
        if (Object.keys(constraints).length) await nextTrack.applyConstraints(constraints);
      }
      verifiedModes = await this._selectStartupMode(nextTrack, generation);
    } catch (error) {
      stopStream(stream);
      throw error;
    }
    if (generation !== this.generation) {
      stopStream(stream);
      throw new DOMException("Session ended", "AbortError");
    }
    try {
      await this.videoSender?.replaceTrack?.(nextTrack);
    } catch (error) {
      stopStream(stream);
      throw error;
    }
    this.stream = stream;
    this.track = nextTrack;
    this.verifiedModes = verifiedModes;
    this.devices = await this._enumerateDevices();
    stopStream(oldStream);
  }

  async _pulse(generation) {
    if (generation !== this.generation || this.phase !== "live" || !this.sessionId || this.heartbeatPromise) return;
    this.heartbeatPromise = this._heartbeatCycle(generation)
      .catch((error) => {
        if (generation !== this.generation || error?.name === "AbortError") return;
        this.heartbeatFailures += 1;
        const message = describeError(error, "Connection to the local service was interrupted.");
        this.onStatus("starting", "Reconnecting to local service…");
        if (this.heartbeatFailures >= 3) void this._connectionEnded(generation, message);
      })
      .finally(() => { this.heartbeatPromise = null; });
    return this.heartbeatPromise;
  }

  async _heartbeatCycle(generation) {
    const response = await this.api.heartbeat(this._heartbeatBody());
    if (generation !== this.generation || this.phase !== "live") return;
    this.heartbeatFailures = 0;
    if (response?.status === "stopped") {
      await this._finishLocal(response.error || "Camera session stopped on desktop", { notifyServer: false });
      this.onStatus("stopped", response.error || "Camera session stopped on desktop");
      this.onChange({ phase: "stopped", applied: null, capabilities: this.capabilities });
      return;
    }
    if (response?.pending?.commandId && response.pending.sessionId === this.sessionId) {
      await this._applyRemoteCommand(response.pending, generation);
    } else if (response?.error) {
      this.onStatus("live", response.error);
    }
  }

  _heartbeatBody(extra = {}) {
    return {
      sessionId: this.sessionId,
      capabilities: this.capabilities,
      applied: this.applied,
      ...extra,
    };
  }

  async _applyRemoteCommand(command, generation) {
    const commandId = command.commandId;
    if (!commandId || generation !== this.generation) return;
    if (this.pendingCommandId === commandId) {
      await this._ackRemoteCommand(commandId, generation);
      return;
    }
    if (this.pendingCommandId) return;
    this.pendingCommandId = commandId;
    this.pendingCommandError = "";
    this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices, pendingCommandId: commandId });
    let commandError = "";
    try {
      if (command.controls?.stop === true) {
        await this._finishLocal("Camera stopped from desktop", { notifyServer: false });
        this.onStatus("stopped", "Camera stopped from desktop");
        this.onChange({ phase: "stopped", applied: null, capabilities: this.capabilities });
        return;
      }
      await this._applyControls(command.controls || {}, generation, { remote: true });
    } catch (error) {
      if (generation !== this.generation || error?.name === "AbortError") return;
      commandError = describeError(error, "The phone camera rejected that setting.");
      this.lastError = commandError;
    }
    if (generation !== this.generation || this.phase !== "live") return;
    this.pendingCommandError = commandError;
    await this._ackRemoteCommand(commandId, generation);
  }

  async _ackRemoteCommand(commandId, generation) {
    if (generation !== this.generation || this.phase !== "live" || this.pendingCommandId !== commandId) return;
    const commandError = this.pendingCommandError;
    const response = await this.api.heartbeat(this._heartbeatBody({
      commandId,
      ...(commandError ? { error: commandError } : {}),
    }));
    if (generation !== this.generation) return;
    if (response?.status === "stopped") {
      await this._finishLocal(response.error || "Camera session stopped on desktop", { notifyServer: false });
      this.onStatus("stopped", response.error || "Camera session stopped on desktop");
      this.onChange({ phase: "stopped", applied: null, capabilities: this.capabilities });
      return;
    }
    if (response?.pending?.commandId === commandId) return;
    this.pendingCommandId = "";
    this.pendingCommandError = "";
    this.onStatus("live", commandError || "Camera streaming");
    this.onChange({ phase: this.phase, applied: this.applied, capabilities: this.capabilities, devices: this.devices });
  }

  async _finishLocal(reason, { notifyServer }) {
    const sessionId = this.sessionId;
    const stream = this.stream;
    const peer = this.peer;
    const timer = this.heartbeatTimer;
    const disconnectTimer = this.disconnectTimer;
    this.generation += 1;
    this.phase = "stopped";
    this.heartbeatTimer = null;
    this.disconnectTimer = null;
    this.sessionId = "";
    this.stream = null;
    this.track = null;
    this.peer = null;
    this.videoSender = null;
    this.applied = null;
    this.pendingCommandId = "";
    this.pendingCommandError = "";
    if (timer !== null) this.clearInterval(timer);
    if (disconnectTimer !== null) this.clearTimeout(disconnectTimer);
    if (this.visibilityHandler) this.document?.removeEventListener?.("visibilitychange", this.visibilityHandler);
    this.visibilityHandler = null;
    await this._releaseWakeLock();
    try { peer?.close?.(); } catch { /* Already closed. */ }
    stopStream(stream);
    if (notifyServer && sessionId) {
      void this.api.stop({ sessionId }).catch(() => {});
    }
    this.lastError = reason || "";
  }
}

/** Keep the local monitor detached in dim mode while the WebRTC sender stays live. */
export function syncLocalPreview(video, stream, phase, applied) {
  if (!video) return;
  if (["prepared", "live"].includes(phase) && !applied?.screenDimmed && stream) {
    if (video.srcObject !== stream) video.srcObject = stream;
    try { video.play?.()?.catch?.(() => {}); } catch { /* Autoplay may be denied. */ }
    return;
  }
  if (video.srcObject) {
    try { video.pause?.(); } catch { /* The preview can already be paused. */ }
    video.srcObject = null;
  }
}

function dom() {
  const byId = (id) => document.getElementById(id);
  return {
    status: byId("phone-status"), error: byId("phone-error"),
    video: byId("phone-preview-video"), preview: byId("phone-preview-placeholder"),
    previewFrame: byId("phone-preview-video")?.parentElement,
    formatBadge: byId("phone-format-badge"), cameraWrap: byId("phone-camera-wrap"), camera: byId("phone-camera"),
    formatWrap: byId("phone-format-wrap"), format: byId("phone-format"), fpsWrap: byId("phone-fps-wrap"), fps: byId("phone-fps"), fpsValue: byId("phone-fps-value"),
    zoomWrap: byId("phone-zoom-wrap"), zoom: byId("phone-zoom"), zoomValue: byId("phone-zoom-value"),
    exposureWrap: byId("phone-exposure-wrap"), exposure: byId("phone-exposure"), exposureValue: byId("phone-exposure-value"),
    torchWrap: byId("phone-torch-wrap"), torch: byId("phone-torch"),
    mirror: byId("phone-mirror"), dim: byId("phone-dim"),
    start: byId("phone-start"), stop: byId("phone-stop"),
    overlay: byId("phone-dim-overlay"), reveal: byId("phone-reveal"), dimStop: byId("phone-dim-stop"),
  };
}

function setStatus(elements, state, message) {
  elements.status.textContent = message;
  elements.status.dataset.state = state;
}

function setError(elements, message = "") {
  elements.error.textContent = message;
  elements.error.hidden = !message;
}

function syncPhoneControls(elements, { phase, applied, capabilities, devices = [], error, pendingCommandId }) {
  const live = phase === "live";
  const track = devices.length ? devices : capabilities.cameras;
  const currentCameras = capabilities.cameras || [];
  elements.cameraWrap.hidden = currentCameras.length < 2;
  elements.camera.replaceChildren();
  if (!currentCameras.length) {
    elements.camera.add(new Option(live ? "Camera list unavailable" : "Detecting cameras…", ""));
    elements.camera.disabled = true;
  } else {
    for (const camera of currentCameras) {
      const option = new Option(camera.label || "Camera", camera.id);
      elements.camera.add(option);
    }
    elements.camera.value = applied?.cameraId || currentCameras[0].id;
    elements.camera.disabled = !live || currentCameras.length < 2;
  }

  const active = currentCameras.find((camera) => camera.id === (applied?.cameraId || elements.camera.value));
  const modes = active?.modes || [];
  elements.formatWrap.hidden = modes.length === 0;
  const selectedFormat = `${applied?.width || 0}x${applied?.height || 0}`;
  elements.format.replaceChildren();
  const actualIsPreset = modes.some((mode) => `${mode.width}x${mode.height}` === selectedFormat);
  if (applied && modes.length && !actualIsPreset) {
    const actualOption = new Option(`Current · ${applied.width}×${applied.height}`, selectedFormat);
    actualOption.disabled = true;
    elements.format.add(actualOption);
  }
  if (modes.length) {
    for (const mode of modes) {
      const option = new Option(`${mode.width === 1280 ? "720p" : "1080p"} · ${Math.round(mode.fps)} fps`, `${mode.width}x${mode.height}`);
      elements.format.add(option);
    }
    if (applied && [...elements.format.options].some((option) => option.value === selectedFormat)) {
      elements.format.value = selectedFormat;
    } else if (modes.length) elements.format.value = `${modes[0].width}x${modes[0].height}`;
    elements.format.disabled = !live;
  } else {
    const label = applied ? `Current · ${applied.width}×${applied.height}` : "No supported preset";
    elements.format.add(new Option(label, ""));
    elements.format.disabled = true;
  }

  const cameraRates = active?.frameRates || [];
  const fpsRange = cameraRates.length >= 2
    ? clampRange({ min: cameraRates[0], max: cameraRates[cameraRates.length - 1] })
    : null;
  elements.fpsWrap.hidden = !fpsRange || fpsRange.min === fpsRange.max;
  if (fpsRange) {
    elements.fps.min = String(fpsRange.min);
    elements.fps.max = String(fpsRange.max);
    elements.fps.step = String(fpsRange.step);
    elements.fps.value = String(Math.min(fpsRange.max, Math.max(fpsRange.min, Math.round(applied?.fps || 30))));
    elements.fps.disabled = !live;
  } else {
    elements.fps.disabled = true;
  }
  elements.fpsValue.textContent = applied?.fps ? `${applied.fps.toFixed(1)} fps` : "—";

  syncRange(elements.zoomWrap, elements.zoom, elements.zoomValue, capabilities.zoom, applied?.zoom, live, "×");
  syncRange(elements.exposureWrap, elements.exposure, elements.exposureValue, capabilities.exposureCompensation, applied?.exposure, live, "");
  elements.torchWrap.hidden = !capabilities.torch;
  elements.torch.disabled = !live || !capabilities.torch;
  elements.torch.checked = Boolean(applied?.torch);
  elements.mirror.checked = Boolean(applied?.previewMirrored);
  elements.dim.checked = Boolean(applied?.screenDimmed);
  elements.mirror.disabled = !live;
  elements.dim.disabled = !live;
  const controlsDisabled = !live || Boolean(pendingCommandId);
  elements.camera.disabled = !live || controlsDisabled || currentCameras.length < 2;
  elements.format.disabled = !live || controlsDisabled || modes.length === 0;
  elements.fps.disabled = controlsDisabled || elements.fpsWrap.hidden;
  elements.zoom.disabled = controlsDisabled || elements.zoomWrap.hidden;
  elements.exposure.disabled = controlsDisabled || elements.exposureWrap.hidden;
  elements.torch.disabled = controlsDisabled || !capabilities.torch;
  elements.mirror.disabled = controlsDisabled || !live;
  elements.dim.disabled = controlsDisabled || !live;
  elements.start.disabled = phase === "starting" || live;
  elements.stop.disabled = !live && phase !== "starting";
  elements.formatBadge.textContent = live && applied
    ? `${applied.width} × ${applied.height} / ${applied.fps.toFixed(1)} FPS`
    : phase === "starting" ? "CONNECTING" : "NOT STREAMING";
  elements.preview.hidden = live;
  elements.previewFrame.classList.toggle("is-mirrored", Boolean(applied?.previewMirrored));
  elements.overlay.hidden = !(live && applied?.screenDimmed);
  document.body.classList.toggle("phone-dimmed", live && Boolean(applied?.screenDimmed));
  if (error) setError(elements, error);
  else if (live || phase === "stopped") setError(elements, "");
}

function syncRange(wrap, input, output, rangeValue, appliedValue, live, suffix) {
  const range = getNumericRange(rangeValue);
  wrap.hidden = !range;
  input.disabled = !live || !range;
  if (!range) return;
  input.min = String(range.min);
  input.max = String(range.max);
  input.step = String(range.step || 0.1);
  const value = appliedValue ?? range.min;
  input.value = String(value);
  output.textContent = `${Number(value).toFixed(1)}${suffix}`;
}

function initPhone() {
  initAccentControl();
  revealRole(document, "phone");
  const elements = dom();
  const token = takeBearer("phone");
  if (!token) {
    setStatus(elements, "error", "Open this page from the OmaCam camera link.");
    elements.start.disabled = true;
    setError(elements, "This page has no phone session token.");
    return;
  }
  let api;
  try { api = new BrowserApi(token); }
  catch (error) { setStatus(elements, "error", describeError(error)); setError(elements, error.message); return; }

  const session = new PhoneSession(api, {
    onStatus: (state, message) => setStatus(elements, state, message),
    onChange: (state) => {
      syncLocalPreview(elements.video, session.stream, state.phase, state.applied);
      syncPhoneControls(elements, state);
    },
  });
  const selectedControls = () => {
    const [width, height] = elements.format.value.split("x").map(Number);
    return {
      ...(Number.isInteger(width) && Number.isInteger(height) ? { width, height } : {}),
      fps: Number(elements.fps.value) || undefined,
      zoom: elements.zoom.disabled ? undefined : Number(elements.zoom.value),
      exposure: elements.exposure.disabled ? undefined : Number(elements.exposure.value),
      torch: elements.torch.disabled ? undefined : elements.torch.checked,
    };
  };
  elements.start.addEventListener("click", () => { void session.start({ cameraId: elements.camera.value }); });
  const stop = () => { void session.stop(); };
  elements.stop.addEventListener("click", stop);
  elements.dimStop.addEventListener("click", stop);
  elements.reveal.addEventListener("click", () => { void session.reveal(); });
  elements.camera.addEventListener("change", () => {
    if (session.live) void session.applyLocalControls({ ...selectedControls(), cameraId: elements.camera.value });
  });
  elements.format.addEventListener("change", () => {
    const [width, height] = elements.format.value.split("x").map(Number);
    if (session.live && width && height) void session.applyLocalControls({ width, height, fps: Number(elements.fps.value) || 30 });
  });
  elements.fps.addEventListener("change", () => {
    elements.fpsValue.textContent = `${elements.fps.value} fps`;
    if (session.live) void session.applyLocalControls({ fps: Number(elements.fps.value) });
  });
  elements.zoom.addEventListener("change", () => {
    elements.zoomValue.textContent = `${Number(elements.zoom.value).toFixed(1)}×`;
    if (session.live) void session.applyLocalControls({ zoom: Number(elements.zoom.value) });
  });
  elements.exposure.addEventListener("change", () => {
    elements.exposureValue.textContent = Number(elements.exposure.value).toFixed(1);
    if (session.live) void session.applyLocalControls({ exposure: Number(elements.exposure.value) });
  });
  elements.torch.addEventListener("change", () => {
    if (session.live) void session.applyLocalControls({ torch: elements.torch.checked });
  });
  elements.mirror.addEventListener("change", () => {
    if (session.live) void session.applyLocalControls({ previewMirrored: elements.mirror.checked });
  });
  elements.dim.addEventListener("change", () => {
    if (session.live) void session.applyLocalControls({ screenDimmed: elements.dim.checked });
  });
}

const isDesktopPath = typeof window !== "undefined" && window.location.pathname.replace(/\/$/, "") === "/desktop";
if (typeof document !== "undefined" && !isDesktopPath) initPhone();
