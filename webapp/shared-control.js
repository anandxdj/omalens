const API_ROOT = "/api";

export class BrowserApiError extends Error {
  constructor(status, message) {
    super(message || `Request failed (${status})`);
    this.name = "BrowserApiError";
    this.status = status;
  }
}

/** Reveal only the page shell that owns this route before auth can fail. */
export function revealRole(documentRef = globalThis.document, role) {
  if (role !== "phone" && role !== "desktop") throw new Error(`Unknown OmaCam page role: ${role}`);
  const phone = documentRef?.getElementById?.("phone-app");
  const desktop = documentRef?.getElementById?.("desktop-app");
  if (phone) phone.hidden = role !== "phone";
  if (desktop) desktop.hidden = role !== "desktop";
}

/** Read the role-specific bearer from the URL fragment and remove it from history. */
export function takeBearer(role, windowRef = globalThis.window) {
  if (!windowRef?.location) return "";
  const raw = windowRef.location.hash.replace(/^#/, "");
  const token = role === "desktop"
    ? new URLSearchParams(raw).get("token") || ""
    : raw.startsWith("token=") ? new URLSearchParams(raw).get("token") || "" : raw;
  if (token && windowRef.history?.replaceState) {
    windowRef.history.replaceState(null, "", `${windowRef.location.pathname}${windowRef.location.search}`);
  }
  return token;
}

export class BrowserApi {
  constructor(token, fetchImpl = globalThis.fetch) {
    if (!token) throw new Error("This page is missing its session token. Reopen it from the OmaCam link.");
    if (typeof fetchImpl !== "function") throw new Error("This browser does not provide fetch().");
    this.token = token;
    this.fetch = fetchImpl;
  }

  async request(path, { method = "GET", body, signal, headers = {} } = {}) {
    const requestHeaders = new Headers(headers);
    requestHeaders.set("Authorization", `Bearer ${this.token}`);
    if (body !== undefined) requestHeaders.set("Content-Type", "application/json");
    let response;
    try {
      response = await this.fetch(`${API_ROOT}${path}`, {
        method,
        headers: requestHeaders,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal,
        cache: "no-store",
        credentials: "same-origin",
        redirect: "error",
      });
    } catch (error) {
      if (error?.name === "AbortError") throw error;
      throw new Error(error?.message || "The local OmaCam service could not be reached.");
    }

    if (response.status === 204) return null;
    const contentType = response.headers?.get?.("content-type") || "";
    if (!response.ok) {
      let message = "";
      try {
        if (contentType.includes("json")) {
          const payload = await response.json();
          message = payload?.message || payload?.error || "";
        } else message = (await response.text()).trim();
      } catch { /* Keep the status based fallback. */ }
      throw new BrowserApiError(response.status, message || `Local service returned ${response.status}.`);
    }
    if (contentType.includes("application/json")) return response.json();
    if (contentType.includes("image/")) return response.blob();
    const text = await response.text();
    if (!text) return null;
    try { return JSON.parse(text); } catch { return text; }
  }

  offer(body, options) { return this.request("/offer", { method: "POST", body, ...options }); }
  getState(options) { return this.request("/state", options); }
  command(body, options) { return this.request("/command", { method: "POST", body, ...options }); }
  heartbeat(body, options) { return this.request("/heartbeat", { method: "POST", body, ...options }); }
  stop(body, options) { return this.request("/stop", { method: "POST", body, ...options }); }

  async preview(options) {
    return this.request("/preview", options);
  }
}

export function readCapabilities(track) {
  try { return track?.getCapabilities?.() || {}; }
  catch { return {}; }
}

export function readSettings(track) {
  try { return track?.getSettings?.() || {}; }
  catch { return {}; }
}

function finite(value) {
  return typeof value === "number" && Number.isFinite(value);
}

export function getNumericRange(value) {
  if (!value || !finite(value.min) || !finite(value.max) || value.min > value.max) return null;
  const step = finite(value.step) && value.step > 0 ? value.step : null;
  return { min: value.min, max: value.max, step };
}

function includes(range, value) {
  return Boolean(range && range.min <= value && value <= range.max);
}

export function supportedFormats(capabilities) {
  const width = getNumericRange(capabilities?.width);
  const height = getNumericRange(capabilities?.height);
  const frameRate = getNumericRange(capabilities?.frameRate);
  if (!width || !height || !includes(frameRate, 30)) return [];
  return [
    { width: 1280, height: 720, fps: 30, label: "720p · 30 fps" },
    { width: 1920, height: 1080, fps: 30, label: "1080p · 30 fps" },
  ].filter((mode) => includes(width, mode.width) && includes(height, mode.height));
}

function facingFromDevice(device) {
  const label = `${device?.label || ""} ${device?.facingMode || ""}`.toLowerCase();
  if (/(front|user|selfie)/.test(label)) return "user";
  if (/(back|rear|environment)/.test(label)) return "environment";
  return "unknown";
}

export function buildCameraCapabilities(devices, activeDeviceId, trackCapabilities, settings = {}, verifiedModes = null) {
  const modes = (verifiedModes ?? supportedFormats(trackCapabilities))
    .map(({ width, height, fps }) => ({ width, height, fps }));
  const frameRange = getNumericRange(trackCapabilities?.frameRate);
  const frameRates = frameRange
    ? [...new Set([frameRange.min, ...(finite(settings.frameRate) ? [settings.frameRate] : []), frameRange.max])].sort((a, b) => a - b)
    : [];
  const cameras = (devices || [])
    .filter((device) => device?.kind === "videoinput" && device.deviceId)
    .map((device, index) => ({
      id: device.deviceId,
      label: device.label?.trim() || `Camera ${index + 1}`,
      facing: facingFromDevice(device),
      modes: device.deviceId === activeDeviceId ? modes : [],
      frameRates: device.deviceId === activeDeviceId ? frameRates : [],
    }));
  if (activeDeviceId && !cameras.some((camera) => camera.id === activeDeviceId)) {
    cameras.push({
      id: activeDeviceId,
      label: "Active camera",
      facing: "unknown",
      modes,
      frameRates,
    });
  }

  return {
    cameras,
    zoom: getNumericRange(trackCapabilities?.zoom),
    exposureCompensation: getNumericRange(trackCapabilities?.exposureCompensation),
    torch: trackCapabilities?.torch === true,
    focusModes: Array.isArray(trackCapabilities?.focusMode) ? trackCapabilities.focusMode : [],
    screenDimSupported: true,
  };
}

/** Return camera values read back from MediaStreamTrack.getSettings(). */
export function readAppliedState(track, fallback = {}) {
  const settings = readSettings(track);
  const cameraId = settings.deviceId || fallback.cameraId || "";
  const width = finite(settings.width) ? Math.round(settings.width) : 0;
  const height = finite(settings.height) ? Math.round(settings.height) : 0;
  const fps = finite(settings.frameRate) ? settings.frameRate : 0;
  if (!cameraId || width <= 0 || height <= 0 || fps <= 0) return null;
  return {
    cameraId,
    width,
    height,
    fps,
    zoom: finite(settings.zoom) ? settings.zoom : (fallback.zoom ?? null),
    exposure: finite(settings.exposureCompensation)
      ? settings.exposureCompensation
      : (fallback.exposure ?? null),
    torch: typeof settings.torch === "boolean" ? settings.torch : Boolean(fallback.torch),
    previewMirrored: Boolean(fallback.previewMirrored),
    screenDimmed: Boolean(fallback.screenDimmed),
  };
}

export function offerFormatFromApplied(applied) {
  if (!applied) throw new Error("The camera did not report its actual capture settings.");
  const fps = Math.round(applied.fps);
  if (!Number.isInteger(applied.width) || !Number.isInteger(applied.height)
      || applied.width < 320 || applied.height < 240
      || applied.width % 2 !== 0 || applied.height % 2 !== 0
      || !Number.isInteger(fps) || fps < 15 || fps > 60) {
    throw new Error("The camera reported a format OmaCam cannot send.");
  }
  return { width: applied.width, height: applied.height, fps };
}

export function makeTrackConstraints(controls = {}, capabilities = {}) {
  const constraints = {};
  if (controls.width != null) constraints.width = { exact: controls.width };
  if (controls.height != null) constraints.height = { exact: controls.height };
  if (controls.fps != null) constraints.frameRate = { exact: controls.fps };
  const advanced = {};
  if (controls.zoom != null && getNumericRange(capabilities.zoom)) advanced.zoom = controls.zoom;
  if (controls.exposure != null && getNumericRange(capabilities.exposureCompensation)) {
    advanced.exposureCompensation = controls.exposure;
  }
  if (controls.torch != null && capabilities.torch === true) advanced.torch = controls.torch;
  if (Object.keys(advanced).length > 0) constraints.advanced = [advanced];
  return constraints;
}

export function createCommandId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  const random = Math.random().toString(36).slice(2);
  return `cmd-${Date.now().toString(36)}-${random}`;
}

export function describeError(error, fallback = "The camera session encountered an error.") {
  const message = typeof error?.message === "string" ? error.message.trim() : "";
  return message || fallback;
}

export function initAccentControl(documentRef = globalThis.document, storage = globalThis.localStorage) {
  const input = documentRef?.getElementById?.("accent-picker");
  if (!input || input.dataset.accentReady) return;
  input.dataset.accentReady = "true";
  const apply = (value) => {
    if (!/^#[0-9a-f]{6}$/i.test(value || "")) return;
    documentRef.documentElement.style.setProperty("--accent", value);
    input.value = value;
  };
  try { apply(storage?.getItem("omarchyAccent") || input.value); }
  catch { apply(input.value); }
  input.addEventListener("input", () => {
    apply(input.value);
    try { storage?.setItem("omarchyAccent", input.value); } catch { /* Storage may be disabled. */ }
  });
}

export async function waitForIceGathering(peerConnection, timeoutMs = 10_000) {
  if (peerConnection.iceGatheringState === "complete") return;
  await new Promise((resolve, reject) => {
    let settled = false;
    const finish = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      peerConnection.removeEventListener?.("icegatheringstatechange", onState);
      error ? reject(error) : resolve();
    };
    const onState = () => {
      if (peerConnection.iceGatheringState === "complete") finish();
    };
    const timer = setTimeout(() => finish(new Error("Network setup timed out while preparing the local camera link.")), timeoutMs);
    peerConnection.addEventListener?.("icegatheringstatechange", onState);
    if (peerConnection.iceGatheringState === "complete") finish();
  });
}
