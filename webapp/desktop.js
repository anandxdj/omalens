import {
  BrowserApi,
  BrowserApiError,
  createCommandId,
  describeError,
  initAccentControl,
  revealRole,
  takeBearer,
} from "./shared-control.js";

const STATE_POLL_MS = 1_000;
const PREVIEW_POLL_MS = 500;

function clampIntegerRange(values) {
  if (!Array.isArray(values) || values.length < 2) return null;
  const min = Math.max(15, Math.ceil(Number(values[0])));
  const max = Math.min(60, Math.floor(Number(values[values.length - 1])));
  return Number.isFinite(min) && Number.isFinite(max) && min <= max ? { min, max } : null;
}

export class DesktopControlRoom {
  constructor(api, options = {}) {
    this.api = api;
    this.document = options.document ?? globalThis.document;
    this.setInterval = options.setInterval ?? globalThis.setInterval;
    this.clearInterval = options.clearInterval ?? globalThis.clearInterval;
    this.createObjectURL = options.createObjectURL ?? ((blob) => URL.createObjectURL(blob));
    this.revokeObjectURL = options.revokeObjectURL ?? ((url) => URL.revokeObjectURL(url));
    this.state = null;
    this.selectedCameraId = "";
    this.pendingLocal = false;
    this.stopPending = false;
    this.stateInFlight = false;
    this.previewInFlight = false;
    this.stateTimer = null;
    this.previewTimer = null;
    this.currentPreviewUrl = "";
    this.previewSequence = 0;
    this.stopped = false;
    this.elements = this._elements();
  }

  start() {
    if (this.stateTimer || this.stopped) return;
    void this.refreshState();
    void this.refreshPreview();
    this.stateTimer = this.setInterval(() => { void this.refreshState(); }, STATE_POLL_MS);
    this.previewTimer = this.setInterval(() => {
      if (this.document?.visibilityState !== "hidden") void this.refreshPreview();
    }, PREVIEW_POLL_MS);
  }

  dispose() {
    this.stopped = true;
    if (this.stateTimer !== null) this.clearInterval(this.stateTimer);
    if (this.previewTimer !== null) this.clearInterval(this.previewTimer);
    this.stateTimer = null;
    this.previewTimer = null;
    this._clearPreview();
  }

  async refreshState() {
    if (this.stateInFlight || this.stopped) return;
    this.stateInFlight = true;
    try {
      const state = await this.api.getState();
      if (!state || this.stopped) return;
      this.state = state;
      if (state.applied?.cameraId) this.selectedCameraId = state.pending?.controls?.cameraId || state.applied.cameraId;
      this._renderState();
      if (state.status === "stopped") this._finishStopped();
    } catch (error) {
      if (this.stopped) return;
      this._setStatus("error", "Service unavailable");
      this._setError(describeError(error, "Could not read the local camera session."));
    } finally {
      this.stateInFlight = false;
    }
  }

  async refreshPreview() {
    if (this.previewInFlight || this.stopped || this.document?.visibilityState === "hidden") return;
    this.previewInFlight = true;
    try {
      const blob = await this.api.preview();
      if (this.stopped) return;
      if (blob instanceof Blob) this._showPreview(blob);
      else this._clearPreview();
    } catch (error) {
      if (!this.stopped) {
        this._clearPreview();
        if (error instanceof BrowserApiError && error.status === 401) {
          this._setError("Desktop session token was rejected. Reopen the control room from the local service.");
        }
      }
    } finally {
      this.previewInFlight = false;
    }
  }

  async sendControls(controls) {
    const state = this.state;
    if (!state || state.status !== "live" || state.pending || this.pendingLocal || this.stopPending) return false;
    this.pendingLocal = true;
    this._setCommandNote("Sending to phone…");
    this._renderControls();
    const command = {
      sessionId: state.sessionId,
      commandId: createCommandId(),
      expectedRevision: state.revision,
      controls,
    };
    try {
      const next = await this.api.command(command);
      if (next) {
        this.state = next;
        this.selectedCameraId = controls.cameraId || next.applied?.cameraId || this.selectedCameraId;
        this._renderState();
      }
      this._setCommandNote(next?.pending ? "Waiting for phone acknowledgement…" : "Command accepted.");
      void this.refreshState();
      return true;
    } catch (error) {
      if (error instanceof BrowserApiError && error.status === 409) {
        this._setCommandNote("Camera state changed. Refreshed the latest state.");
        void this.refreshState();
      } else {
        this._setCommandNote("Command was not accepted.");
        this._setError(describeError(error, "The local service rejected the camera command."));
      }
      return false;
    } finally {
      this.pendingLocal = false;
      this._renderControls();
    }
  }

  async stopSession() {
    const state = this.state;
    if (!state?.sessionId || state.status !== "live" || this.stopPending) return false;
    this.stopPending = true;
    this._setStatus("starting", "Stopping camera session…");
    this._renderControls();
    try {
      const stopped = await this.api.stop({ sessionId: state.sessionId });
      if (stopped) this.state = stopped;
      this._renderState();
      if (stopped?.status === "stopped") this._finishStopped();
      return true;
    } catch (error) {
      this._setError(describeError(error, "Stop request did not reach the local service."));
      void this.refreshState();
      return false;
    } finally {
      this.stopPending = false;
      this._renderControls();
    }
  }

  _elements() {
    const byId = (id) => this.document.getElementById(id);
    return {
      status: byId("desktop-status"),
      stop: byId("desktop-stop"),
      monitorState: byId("desktop-monitor-state"),
      formatBadge: byId("desktop-format-badge"),
      preview: byId("desktop-preview"),
      previewEmpty: byId("desktop-preview-empty"),
      previewMessage: byId("desktop-preview-message"),
      monitor: byId("desktop-monitor"),
      monitorFooter: byId("desktop-monitor")?.parentElement?.querySelector(".monitor-footer"),
      linkLabel: byId("desktop-link-label"),
      latencyLabel: byId("desktop-latency-label"),
      cameraWrap: byId("desktop-camera-wrap"),
      camera: byId("desktop-camera"),
      formatWrap: byId("desktop-format-wrap"),
      format: byId("desktop-format"),
      fpsWrap: byId("desktop-fps-wrap"),
      fps: byId("desktop-fps"),
      fpsValue: byId("desktop-fps-value"),
      zoomWrap: byId("desktop-zoom-wrap"),
      zoom: byId("desktop-zoom"),
      zoomValue: byId("desktop-zoom-value"),
      exposureWrap: byId("desktop-exposure-wrap"),
      exposure: byId("desktop-exposure"),
      exposureValue: byId("desktop-exposure-value"),
      torchWrap: byId("desktop-torch-wrap"),
      torch: byId("desktop-torch"),
      mirrorWrap: byId("desktop-mirror-wrap"),
      mirror: byId("desktop-mirror"),
      dimWrap: byId("desktop-dim-wrap"),
      dim: byId("desktop-dim"),
      commandNote: byId("desktop-command-note"),
      sessionState: byId("desktop-session-state"),
      revision: byId("desktop-revision"),
      cameraLabel: byId("desktop-camera-label"),
      error: byId("desktop-error"),
    };
  }

  _renderState() {
    const state = this.state || {};
    const live = state.status === "live";
    const pending = Boolean(state.pending || this.pendingLocal);
    const statusText = live ? "Phone streaming" : state.status === "stopped" ? "Session stopped" : "Waiting for phone";
    this._setStatus(live ? "live" : state.status === "stopped" ? "stopped" : "waiting", statusText);
    this.elements.stop.disabled = !live || this.stopPending;
    this.elements.monitorState.dataset.live = String(live);
    this.elements.monitorState.innerHTML = `<span aria-hidden="true"></span>${live ? "LIVE" : "NO SIGNAL"}`;
    this.elements.sessionState.textContent = String(state.status || "waiting").toUpperCase();
    this.elements.revision.textContent = Number.isFinite(state.revision) ? String(state.revision) : "—";
    const applied = state.applied;
    const camera = state.capabilities?.cameras?.find((item) => item.id === applied?.cameraId);
    this.elements.cameraLabel.textContent = camera?.label || "—";
    this.elements.formatBadge.textContent = applied
      ? `${applied.width} × ${applied.height} / ${Number(applied.fps).toFixed(1)} FPS`
      : "—";
    this.elements.monitorFooter.dataset.live = String(live);
    this.elements.linkLabel.textContent = live ? "PHONE LINK ACTIVE" : "LOCAL LINK STANDBY";
    this.elements.latencyLabel.textContent = live ? "SHARED DECODER PREVIEW" : "FRAME —";
    this._setError(state.error || "");
    if (live && applied) this.elements.monitor.classList.toggle("is-mirrored", Boolean(applied.previewMirrored));
    else this.elements.monitor.classList.remove("is-mirrored");
    if (state.status === "stopped") this._clearPreview();
    if (state.pending) this._setCommandNote("Applying command on phone…");
    else if (!live && !state.error) this._setCommandNote("Phone camera capabilities appear when streaming.");
    this._renderControls(pending);
  }

  _renderControls(pending = Boolean(this.state?.pending || this.pendingLocal)) {
    const state = this.state || {};
    const capabilities = state.capabilities || {};
    const applied = state.applied;
    const live = state.status === "live";
    const disabled = !live || pending || this.stopPending;
    const cameras = Array.isArray(capabilities.cameras) ? capabilities.cameras : [];
    this.elements.cameraWrap.hidden = cameras.length < 2;
    this._fillSelect(this.elements.camera, cameras.map((camera) => [camera.id, camera.label || "Camera"]), this.selectedCameraId || applied?.cameraId || "");
    this.elements.camera.disabled = disabled || cameras.length < 2;

    const selectedCamera = cameras.find((camera) => camera.id === (this.selectedCameraId || applied?.cameraId));
    const modes = selectedCamera?.modes || [];
    this.elements.formatWrap.hidden = modes.length === 0;
    this.elements.format.replaceChildren();
    const currentFormat = applied ? `${applied.width}x${applied.height}` : "";
    const currentIsMode = modes.some((mode) => `${mode.width}x${mode.height}` === currentFormat);
    if (applied && !currentIsMode && applied.cameraId === selectedCamera?.id) {
      const actual = new Option(`Current · ${applied.width}×${applied.height}`, currentFormat);
      actual.disabled = true;
      this.elements.format.add(actual);
    }
    for (const mode of modes) {
      const label = `${mode.width === 1280 ? "720p" : mode.width === 1920 ? "1080p" : `${mode.width}×${mode.height}`} · ${Number(mode.fps).toFixed(0)} fps`;
      this.elements.format.add(new Option(label, `${mode.width}x${mode.height}`));
    }
    if (applied && applied.cameraId === selectedCamera?.id
        && [...this.elements.format.options].some((option) => option.value === currentFormat)) {
      this.elements.format.value = currentFormat;
    }
    this.elements.format.disabled = disabled || modes.length === 0;

    const rates = selectedCamera?.frameRates || [];
    const fpsRange = clampIntegerRange(rates);
    this.elements.fpsWrap.hidden = !fpsRange || fpsRange.min === fpsRange.max;
    if (fpsRange) {
      this.elements.fps.min = String(fpsRange.min);
      this.elements.fps.max = String(fpsRange.max);
      this.elements.fps.step = "1";
      this.elements.fps.value = String(Math.min(fpsRange.max, Math.max(fpsRange.min, Math.round(applied?.fps || fpsRange.min))));
      this.elements.fps.disabled = disabled;
    } else this.elements.fps.disabled = true;
    this.elements.fpsValue.textContent = applied?.fps ? `${Number(applied.fps).toFixed(1)} fps` : "—";

    this._renderRange(this.elements.zoomWrap, this.elements.zoom, this.elements.zoomValue, capabilities.zoom, applied?.zoom, disabled, "×");
    this._renderRange(this.elements.exposureWrap, this.elements.exposure, this.elements.exposureValue,
      capabilities.exposureCompensation, applied?.exposure, disabled, "");
    this.elements.torchWrap.hidden = capabilities.torch !== true;
    this.elements.torch.disabled = disabled || capabilities.torch !== true;
    this.elements.torch.checked = Boolean(applied?.torch);
    this.elements.mirrorWrap.hidden = false;
    this.elements.mirror.disabled = disabled;
    this.elements.mirror.checked = Boolean(applied?.previewMirrored);
    this.elements.dimWrap.hidden = capabilities.screenDimSupported !== true;
    this.elements.dim.disabled = disabled || capabilities.screenDimSupported !== true;
    this.elements.dim.checked = Boolean(applied?.screenDimmed);
  }

  _fillSelect(select, options, selectedValue) {
    const previous = select.value;
    select.replaceChildren();
    if (!options.length) {
      select.add(new Option("Waiting for phone", ""));
      return;
    }
    for (const [value, label] of options) select.add(new Option(label, value));
    const desired = options.some(([value]) => value === selectedValue) ? selectedValue
      : options.some(([value]) => value === previous) ? previous : options[0][0];
    select.value = desired;
  }

  _renderRange(wrap, input, output, range, value, disabled, suffix) {
    if (!range || !Number.isFinite(Number(range.min)) || !Number.isFinite(Number(range.max)) || range.min > range.max) {
      wrap.hidden = true;
      input.disabled = true;
      return;
    }
    wrap.hidden = false;
    input.min = String(range.min);
    input.max = String(range.max);
    input.step = String(range.step || 0.1);
    const current = value ?? range.min;
    input.value = String(current);
    input.disabled = disabled;
    output.textContent = `${Number(current).toFixed(1)}${suffix}`;
  }

  _showPreview(blob) {
    const url = this.createObjectURL(blob);
    const previous = this.currentPreviewUrl;
    const sequence = ++this.previewSequence;
    this.currentPreviewUrl = url;
    this.elements.preview.hidden = false;
    this.elements.previewEmpty.hidden = true;
    this.elements.preview.onload = () => {
      if (previous) this.revokeObjectURL(previous);
      if (sequence === this.previewSequence) this.elements.latencyLabel.textContent = "SHARED DECODE · FRESH FRAME";
      this.elements.preview.onload = null;
      this.elements.preview.onerror = null;
    };
    this.elements.preview.onerror = () => {
      this.revokeObjectURL(url);
      if (this.currentPreviewUrl === url) this.currentPreviewUrl = "";
      this.elements.preview.hidden = true;
      this.elements.previewEmpty.hidden = false;
      this.elements.previewMessage.textContent = "Decoded preview could not be displayed";
      this.elements.preview.onload = null;
      this.elements.preview.onerror = null;
    };
    this.elements.preview.src = url;
  }

  _clearPreview() {
    this.previewSequence += 1;
    if (this.currentPreviewUrl) this.revokeObjectURL(this.currentPreviewUrl);
    this.currentPreviewUrl = "";
    this.elements.preview.removeAttribute("src");
    this.elements.preview.hidden = true;
    this.elements.previewEmpty.hidden = false;
    this.elements.previewMessage.textContent = this.state?.status === "live"
      ? "Waiting for a fresh decoded frame" : "Waiting for phone camera";
    if (this.elements.latencyLabel) this.elements.latencyLabel.textContent = "FRAME —";
  }

  _finishStopped() {
    this._clearPreview();
    if (this.stateTimer !== null) this.clearInterval(this.stateTimer);
    if (this.previewTimer !== null) this.clearInterval(this.previewTimer);
    this.stateTimer = null;
    this.previewTimer = null;
    this.stopped = true;
  }

  _setStatus(state, message) {
    this.elements.status.textContent = message;
    this.elements.status.dataset.state = state;
  }

  _setError(message) {
    this.elements.error.textContent = message;
    this.elements.error.hidden = !message;
  }

  _setCommandNote(message) {
    this.elements.commandNote.textContent = message;
  }
}

function initDesktop() {
  initAccentControl();
  revealRole(document, "desktop");
  const token = takeBearer("desktop");
  const status = document.getElementById("desktop-status");
  const error = document.getElementById("desktop-error");
  if (!token) {
    status.textContent = "Open the control room link from OmaCam";
    status.dataset.state = "error";
    error.textContent = "This page has no desktop session token.";
    error.hidden = false;
    return;
  }
  let api;
  try { api = new BrowserApi(token); }
  catch (reason) { status.textContent = describeError(reason); status.dataset.state = "error"; return; }
  const room = new DesktopControlRoom(api);
  room.start();

  const element = (id) => document.getElementById(id);
  const send = (controls) => { void room.sendControls(controls); };
  element("desktop-stop").addEventListener("click", () => { void room.stopSession(); });
  element("desktop-camera").addEventListener("change", () => {
    const cameraId = element("desktop-camera").value;
    if (cameraId) send({ cameraId });
  });
  element("desktop-format").addEventListener("change", () => {
    const [width, height] = element("desktop-format").value.split("x").map(Number);
    const camera = room.state?.capabilities?.cameras?.find((item) => item.id === (room.selectedCameraId || room.state?.applied?.cameraId));
    const mode = camera?.modes?.find((item) => item.width === width && item.height === height);
    if (mode) send({ width: mode.width, height: mode.height, fps: mode.fps });
  });
  element("desktop-fps").addEventListener("change", () => {
    if (room.state?.applied) send({ fps: Number(element("desktop-fps").value) });
  });
  element("desktop-zoom").addEventListener("change", () => send({ zoom: Number(element("desktop-zoom").value) }));
  element("desktop-exposure").addEventListener("change", () => send({ exposure: Number(element("desktop-exposure").value) }));
  element("desktop-torch").addEventListener("change", () => send({ torch: element("desktop-torch").checked }));
  element("desktop-mirror").addEventListener("change", () => send({ previewMirrored: element("desktop-mirror").checked }));
  element("desktop-dim").addEventListener("change", () => send({ screenDimmed: element("desktop-dim").checked }));
  window.addEventListener("pagehide", () => room.dispose(), { once: true });
}

const isDesktopPath = typeof window !== "undefined" && window.location.pathname.replace(/\/$/, "") === "/desktop";
if (typeof document !== "undefined" && isDesktopPath) initDesktop();
