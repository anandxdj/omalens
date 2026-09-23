import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "dev.omacam"
  ipcTarget: "dev.omacam"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  property bool openedFromHotkey: false
  property bool checking: false
  property string report: "Open the panel to check this computer."
  property string checkError: ""
  property var snapshot: null
  property string previewFrame: ""
  property string previewError: ""
  property bool previewReceived: false
  property double lastRevision: 0
  property double lastEventRevision: 0
  property string actionError: ""
  property string actionName: ""
  property string diagnosticsReport: ""
  property string diagnosticsError: ""
  property bool diagnosticsExpanded: false
  property bool forgetArmed: false
  property int actionCursor: 0
  readonly property string runtimeDirectory: Quickshell.env("XDG_RUNTIME_DIR") || ""
  readonly property string previewSocket: runtimeDirectory === "" ? "" : runtimeDirectory + "/omacam/preview.sock"
  readonly property var barIdentity: hostWidget || root
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property bool serviceReady: snapshot !== null
  readonly property bool outputReady: serviceReady && snapshot.output && snapshot.output.ready === true
  readonly property string outputState: serviceReady && snapshot.state ? snapshot.state.output : "unknown"
  readonly property string captureState: serviceReady && snapshot.state ? snapshot.state.capture : "unknown"
  readonly property string trustState: serviceReady && snapshot.state ? snapshot.state.trust : "unknown"
  readonly property string connectionState: serviceReady && snapshot.state ? snapshot.state.connection : "unknown"
  readonly property bool startPending: hasPendingOperation("request_start")
  readonly property bool stopPending: hasPendingOperation("stop")
  readonly property bool forgetPending: hasPendingOperation("forget_peer")
  readonly property bool lifecycleBusy: startPending || stopPending || forgetPending
  readonly property bool canStart: serviceReady && outputReady && !actionRequest.running && !lifecycleBusy
    && trustState === "trusted" && connectionState === "online" && captureState === "idle"
  readonly property bool canStop: serviceReady && !actionRequest.running && !forgetPending
    && (captureState !== "idle" || startPending)
  readonly property bool canForget: serviceReady && !actionRequest.running
    && !lifecycleBusy && trustState === "trusted"
  readonly property string statusText: {
    if (checking) return "OmaCam — checking service"
    if (!serviceReady) return "OmaCam — service unavailable"
    if (captureState === "streaming")
      return outputReady ? "OmaCam — camera sharing" : "OmaCam — camera output unavailable"
    if (captureState === "awaiting_consent") return "OmaCam — approval needed"
    if (captureState === "starting") return "OmaCam — starting camera"
    if (captureState === "stopping") return "OmaCam — stopping camera"
    if (captureState === "failed") return "OmaCam — camera error"
    if (startPending) return "OmaCam — approval needed"
    if (stopPending) return "OmaCam — stopping camera"
    if (forgetPending) return "OmaCam — forgetting phone"
    if (trustState !== "trusted") return "OmaCam — pair phone"
    if (connectionState !== "online") return "OmaCam — phone offline"
    if (outputState === "failed") return "OmaCam — camera output unavailable"
    return outputReady ? "OmaCam — ready" : "OmaCam — setup needed"
  }

  function hasPendingOperation(intent) {
    if (!snapshot || !snapshot.operations) return false
    for (var i = 0; i < snapshot.operations.length; i++) {
      var operation = snapshot.operations[i]
      if (operation && operation.intent === intent && operation.state === "pending") return true
    }
    return false
  }

  function actionAvailable(index) {
    if (index === 0) return canStart
    if (index === 1) return canStop
    if (index === 2) return canForget
    if (index === 3) return !checking
    if (index === 4) return !diagnosticsRequest.running
    return false
  }

  function ensureActionCursor() {
    if (actionAvailable(actionCursor)) return
    for (var offset = 1; offset <= 5; offset++) {
      var candidate = (actionCursor + offset) % 5
      if (actionAvailable(candidate)) {
        actionCursor = candidate
        return
      }
    }
  }

  function boundedText(value, limit, fallback) {
    var text = value === undefined || value === null ? "" : String(value)
    if (text === "") return fallback || ""
    return text.slice(0, limit)
  }

  function providerStatus(key) {
    if (!serviceReady || !snapshot.provider) return "unknown"
    return boundedText(snapshot.provider[key], 64, "unknown")
  }

  function providerGuidance() {
    return "Provider order: already-enabled native UVC (" + providerStatus("native_uvc")
      + "), qualified browser/PWA (" + providerStatus("browser")
      + "), then Android companion fallback (" + providerStatus("companion") + ")."
  }

  function selectedProvider() {
    if (!serviceReady || !snapshot.provider) return "none"
    return boundedText(snapshot.provider.selected, 64, "none")
  }

  function operationErrorText() {
    if (!snapshot || !snapshot.last_error) return ""
    var error = snapshot.last_error
    var code = boundedText(error.error_code, 64, "")
    var message = boundedText(error.message, 320, "The last operation failed.")
    return code === "" ? "Last operation failed: " + message
      : "Last operation failed (" + code + "): " + message
  }

  function previewIsActive() {
    return root.opened && root.serviceReady && root.snapshot.preview
      && root.snapshot.preview.active === true && previewSocket !== ""
  }

  function stopPreview() {
    previewRestartTimer.stop()
    preview.running = false
    previewFrame = ""
    previewError = ""
    previewReceived = false
  }

  onOpenedChanged: {
    if (opened) {
      // A reopened panel must not render a snapshot from a previous service
      // instance or revision epoch while the fresh read is in flight.
      snapshot = null
      lastRevision = 0
      lastEventRevision = 0
      stopPreview()
      startPreview()
      stateEvents.running = true
    }
    else {
      stopPreview()
      stateEvents.running = false
      diagnosticsRequest.running = false
      forgetArmed = false
      forgetTimer.stop()
    }
  }

  function open() {
    openedFromHotkey = false
    root.controller.show()
    refresh()
    startPreview()
  }

  function openFromHotkey() {
    openedFromHotkey = true
    root.controller.show()
    refresh()
    startPreview()
  }

  function close() {
    stopPreview()
    root.controller.hide()
  }

  function toggle() {
    if (root.opened) close()
    else openFromHotkey()
  }

  function closeForPopoutSwitch() {
    root.popoutSwitchClosing = true
    close()
    Qt.callLater(function() { root.popoutSwitchClosing = false })
  }

  function switchPanel(direction) {
    if (root.bar && typeof root.bar.switchPanelFrom === "function")
      return root.bar.switchPanelFrom(root.barIdentity, direction)
    return false
  }

  function refresh() {
    if (snapshotRequest.running) return
    checkError = ""
    checking = true
    snapshotRequest.running = true
  }

  function stateSummary() {
    if (!serviceReady) return "The OmaCam session service is unavailable."
    if (startPending) return "Start is pending. Continue on your phone to approve camera sharing, or press Stop to cancel it."
    if (stopPending) return "Stop is pending. Camera sharing is being cancelled and old frames are being cleared."
    if (forgetPending) return "Forget is pending. Desktop trust is being revoked and camera sharing is being stopped."
    if (captureState === "awaiting_consent") return "Continue on your phone to approve camera sharing."
    if (captureState === "starting") return "Starting the authenticated camera session…"
    if (captureState === "streaming") return outputReady
      ? "Camera is sharing to OmaCam Camera."
      : "Phone video is connected, but the virtual camera is unavailable."
    if (captureState === "stopping") return "Stopping camera sharing and clearing old frames…"
    if (captureState === "failed") return "Camera sharing stopped after a failure. Review diagnostics below."
    if (trustState !== "trusted") return "No trusted phone. Pair a supported provider before starting."
    if (connectionState === "recovering") return "Trusted phone disconnected. Reopen the companion to reconnect; camera stays stopped."
    if (connectionState !== "online") return "Trusted phone is offline. Open OmaCam on the phone and choose Find trusted laptop."
    if (outputState === "failed") return "OmaCam Camera is unavailable. Review diagnostics; no other camera is changed."
    if (outputState === "missing") return "Phone is ready, but OmaCam Camera still needs intentional provisioning."
    if (!outputReady) return "OmaCam Camera is unavailable. Review diagnostics; no other camera is changed."
    return "Ready. Start sends a request; the phone must still approve camera sharing."
  }

  function onboardingText() {
    if (!serviceReady)
      return "Install the desktop package, then start its user service when you are ready. Installation must not start it automatically."
    if (outputState === "failed")
      return "OmaCam Camera is unavailable. Run read-only diagnostics and repair only an OmaCam-owned output; existing cameras and system settings are never changed. " + providerGuidance()
    if (outputState === "missing")
      return "Provision one dedicated OmaCam Camera output, then refresh. Existing cameras and system settings are never changed. " + providerGuidance()
    if (!outputReady)
      return "OmaCam Camera is not ready. Run read-only diagnostics before taking any repair action. " + providerGuidance()
    if (trustState !== "trusted") {
      if (trustState === "revoked")
        return "Desktop trust was forgotten. If the phone still remembers this laptop, choose Forget laptop there before pairing again. " + providerGuidance()
      return "No trusted phone. " + providerGuidance() + " Use the approved companion pairing flow to add a trusted phone; pairing never starts the camera."
    }
    if (connectionState !== "online")
      return "Keep both devices on a reachable local network. If connection fails, review firewall access explicitly; OmaCam will not change firewall, routes, DNS, VPN, USB, or hotspot settings."
    return "Selected provider: " + selectedProvider() + ". In your meeting app, select OmaCam Camera. The meeting app keeps control of its microphone and camera selection."
  }

  function moveActionCursor(delta) {
    if (delta === 0) return
    var direction = delta < 0 ? -1 : 1
    var steps = Math.min(Math.abs(delta), 5)
    var candidate = actionCursor
    for (var i = 0; i < steps; i++) {
      candidate = (candidate + direction + 5) % 5
      if (actionAvailable(candidate)) {
        actionCursor = candidate
        return
      }
    }
    ensureActionCursor()
  }

  function scrollContent(delta) {
    if (!panelFlick || delta === 0 || panelFlick.contentHeight <= panelFlick.height) return
    var maximum = Math.max(0, panelFlick.contentHeight - panelFlick.height)
    panelFlick.contentY = Math.max(0, Math.min(maximum, panelFlick.contentY + delta * Style.space(56)))
  }

  function activateActionCursor() {
    if (!actionAvailable(actionCursor)) return
    if (actionCursor === 0) requestAction("start")
    else if (actionCursor === 1) requestAction("stop")
    else if (actionCursor === 2) armOrForget()
    else if (actionCursor === 3) refresh()
    else if (actionCursor === 4) toggleDiagnostics()
  }

  function armOrForget() {
    if (!canForget) {
      forgetArmed = false
      forgetTimer.stop()
      return
    }
    if (!forgetArmed) {
      forgetArmed = true
      forgetTimer.restart()
      return
    }
    forgetArmed = false
    requestAction("forget")
  }

  function toggleDiagnostics() {
    diagnosticsExpanded = !diagnosticsExpanded
    if (!diagnosticsExpanded || diagnosticsRequest.running) return
    diagnosticsError = ""
    diagnosticsReport = ""
    diagnosticsRequest.running = true
  }

  function startPreview() {
    if (!previewIsActive() || preview.running) return
    previewError = ""
    preview.running = true
  }

  function requestAction(name) {
    if (actionRequest.running) return
    if (name === "start" && !canStart) return
    if (name === "stop" && !canStop) return
    if (name === "forget" && !canForget) return
    actionError = ""
    actionName = name
    var operationId = "panel-" + name + "-" + Date.now()
    actionRequest.command = ["omacam-daemon", "ipc", name, operationId]
    actionRequest.running = true
  }

  Process {
    id: snapshotRequest
    command: ["omacam-daemon", "ipc", "snapshot"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var encoded = String(text || "").trim()
        if (encoded === "" || encoded.length > 65536) {
          root.snapshot = null
          root.lastRevision = 0
          root.lastEventRevision = 0
          root.stopPreview()
          root.checkError = encoded === ""
            ? "The OmaCam service returned no snapshot."
            : "The OmaCam service returned an oversized snapshot."
          return
        }
        try {
          var next = JSON.parse(encoded)
          if (!next || next.schema_version !== 2 || next.api_version !== 1)
            throw new Error("unsupported snapshot version")
          var revision = Number(next.revision)
          if (!Number.isFinite(revision) || revision < 0 || Math.floor(revision) !== revision
              || revision > 9007199254740991)
            throw new Error("invalid snapshot revision")
          if (!next.state || typeof next.state.trust !== "string"
              || typeof next.state.connection !== "string"
              || typeof next.state.capture !== "string"
              || typeof next.state.output !== "string"
              || !next.provider || !next.preview || !next.output
              || typeof next.output.ready !== "boolean"
              || typeof next.preview.active !== "boolean"
              || !Array.isArray(next.operations) || next.operations.length > 32)
            throw new Error("incomplete snapshot")
          // A restarted user service begins a new revision epoch. The event
          // stream resets lastRevision before this callback; otherwise a
          // lower revision is stale and must not regress the visible state.
          if (root.lastEventRevision > revision && root.lastEventRevision !== 0)
            throw new Error("snapshot is older than the latest state event")
          root.lastRevision = revision
          root.snapshot = next
          if (next.preview && next.preview.active === true) root.startPreview()
          else root.stopPreview()
          if (root.trustState !== "trusted" || root.forgetPending) {
            root.forgetArmed = false
            root.forgetTimer.stop()
          }
          root.ensureActionCursor()
          var trust = next.state && next.state.trust ? next.state.trust : "unknown"
          var connection = next.state && next.state.connection ? next.state.connection : "unknown"
          var capture = next.state && next.state.capture ? next.state.capture : "unknown"
          root.report = "Service online\nTrust: " + trust + "\nConnection: " + connection + "\nCapture: " + capture
        } catch (error) {
          root.snapshot = null
          root.lastRevision = 0
          root.lastEventRevision = 0
          root.stopPreview()
          root.checkError = "The OmaCam service returned an invalid snapshot."
        }
      }
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {}
    }
    onExited: function(exitCode) {
      root.checking = false
      if (exitCode !== 0) {
        root.snapshot = null
        root.lastRevision = 0
        root.lastEventRevision = 0
        root.stopPreview()
      }
      if (exitCode !== 0 && root.checkError === "")
        root.checkError = "The OmaCam session service is unavailable."
    }
  }

  Process {
    id: stateEvents
    command: ["omacam-daemon", "ipc", "events"]
    stdout: SplitParser {
      onRead: function(line) {
        var revision = Number(String(line || "").trim())
        if (!Number.isFinite(revision) || revision < 1 || Math.floor(revision) !== revision
            || revision > 9007199254740991)
          return
        if (root.lastEventRevision !== 0 && revision < root.lastEventRevision) {
          // A service restart resets the local revision epoch. Drop the old
          // snapshot so a lower revision cannot be rendered as current.
          root.lastRevision = 0
          root.snapshot = null
          root.stopPreview()
        }
        if (revision === root.lastEventRevision) return
        root.lastEventRevision = revision
        // Events are refresh hints, never incremental state. Any gap, reset,
        // or newer revision causes a full bounded snapshot fetch.
        if (root.lastRevision !== revision) root.refresh()
      }
    }
    onExited: function(exitCode) {
      if (root.opened) {
        root.snapshot = null
        root.lastRevision = 0
        root.lastEventRevision = 0
        root.stopPreview()
        root.checkError = "State updates ended; refresh restores the full snapshot."
      }
    }
  }

  Process {
    id: diagnosticsRequest
    command: ["omacam-daemon", "ipc", "diagnostics"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var encoded = String(text || "").trim()
        try {
          if (encoded.length > 65536) throw new Error("oversized diagnostics")
          var result = JSON.parse(encoded)
          if (!result || result.schema_version !== 2) throw new Error("unsupported diagnostics")
          var boundedHost = String(result.host || "").slice(0, 6000)
          var boundedProviders = String(result.providers || "").slice(0, 6000)
          root.diagnosticsReport = boundedHost + "\n\n" + boundedProviders
        } catch (error) {
          root.diagnosticsError = "The service returned invalid diagnostics."
        }
      }
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) {
        root.diagnosticsReport = ""
        root.diagnosticsError = "Diagnostics are unavailable while the service is offline."
      }
    }
  }

  Timer {
    id: forgetTimer
    interval: 8000
    onTriggered: root.forgetArmed = false
  }

  Timer {
    id: previewRestartTimer
    interval: 1000
    repeat: false
    onTriggered: root.startPreview()
  }

  Timer {
    interval: 5000
    running: root.opened
    repeat: true
    onTriggered: {
      root.refresh()
      if (!stateEvents.running) stateEvents.running = true
    }
  }

  Process {
    id: preview
    command: ["omacam-preview", "--socket", root.previewSocket, "--wait-for-socket"]
    stdout: SplitParser {
      onRead: function(line) {
        var frame = String(line || "").trim()
        // The helper emits STANDARD base64 for JPEGs capped at 256 KiB.
        // Reject malformed or oversized lines before handing them to QML's
        // image decoder, keeping preview input bounded and non-networked.
        if (!root.opened || frame.length < 8 || frame.length > 349528
            || frame.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(frame)) return
        if (previewImage.status !== Image.Loading) {
          root.previewFrame = frame
          root.previewReceived = true
          root.previewError = ""
        }
      }
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {}
    }
    onExited: function(exitCode) {
      if (root.opened && exitCode !== 0 && root.previewError === "")
        root.previewError = "Preview is unavailable. Camera output is unaffected."
      root.previewFrame = ""
      root.previewReceived = false
      if (root.opened && root.previewIsActive() && exitCode === 0)
        previewRestartTimer.restart()
    }
  }

  Process {
    id: actionRequest
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {}
    }
    onExited: function(exitCode) {
      if (exitCode !== 0 && root.actionError === "")
        root.actionError = root.actionName === "start"
          ? "Start was rejected. Refresh state, then confirm the phone is trusted, online, and idle."
          : (root.actionName === "forget"
            ? "Forget was rejected. Refresh state and try again."
            : "Stop was rejected. Refresh state; the service may be unavailable.")
      root.refresh()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    centerOnBar: true
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(440))
    contentHeight: panel.fittedContentHeight(content.implicitHeight)

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onMoveRequested: function(dx, dy) {
        if (dx !== 0 || dy !== 0) root.moveActionCursor(dx !== 0 ? dx : dy)
        if (dy !== 0) root.scrollContent(dy)
      }
      onActivateRequested: root.activateActionCursor()
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(key) {
        if (key === "r" || key === "R") root.refresh()
        else if (key === "d" || key === "D") root.toggleDiagnostics()
      }

      Flickable {
        id: panelFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: content.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: content
          width: panelFlick.width
          spacing: Style.space(16)

        Item {
          width: parent.width
          height: width * 9 / 16

          Rectangle {
            anchors.fill: parent
            radius: Style.cornerRadius
            color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.035)
            border.width: 1
            border.color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.1)
          }


          Image {
            id: previewImage
            anchors.fill: parent
            source: root.previewFrame === "" ? "" : "data:image/jpeg;base64," + root.previewFrame
            sourceSize.width: Math.max(1, width)
            sourceSize.height: Math.max(1, height)
            fillMode: Image.PreserveAspectFit
            asynchronous: true
            cache: false
            visible: root.previewFrame !== ""
            Accessible.role: Accessible.Graphic
            Accessible.name: root.previewFrame === "" ? "No camera preview" : "OmaCam camera preview"
            Accessible.description: "Bounded local preview from the same decoded source as OmaCam Camera"
            Accessible.ignored: root.previewFrame === ""
            onStatusChanged: {
              if (status === Image.Error && root.opened)
                root.previewError = "The preview frame could not be decoded. Camera output is unaffected."
            }
          }

          Text {
            anchors.centerIn: parent
            width: parent.width - Style.space(64)
            text: root.checking ? "Checking OmaCam service…"
              : (!root.serviceReady ? "OmaCam service unavailable"
                : (root.previewError !== "" ? "Preview unavailable; camera output unaffected"
                  : (root.outputState === "missing" ? "Camera setup needed"
                    : (root.outputReady ? "Camera output ready" : "Camera output unavailable"))))
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.title
            font.bold: true
            visible: root.previewFrame === "" || previewImage.status === Image.Error
            Accessible.role: Accessible.StaticText
            Accessible.name: text
          }

          Repeater {
            model: [
              { x: Style.space(18), y: Style.space(18), glyph: "⌜" },
              { x: parent.width - Style.space(38), y: Style.space(18), glyph: "⌝" },
              { x: Style.space(18), y: parent.height - Style.space(40), glyph: "⌞" },
              { x: parent.width - Style.space(38), y: parent.height - Style.space(40), glyph: "⌟" }
            ]
            delegate: Text {
              required property var modelData
              x: modelData.x
              y: modelData.y
              text: modelData.glyph
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.display
            }
          }
        }

        Column {
          width: parent.width
          spacing: Style.space(3)

          Text {
            text: "OmaCam session"
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.subtitle
            font.bold: true
          }
          Text {
            text: "Authoritative service state. Opening this panel never starts phone capture."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
          }
        }


        Row {
          width: parent.width
          spacing: Style.space(10)

          PanelActionButton {
            id: startButton
            iconText: "󰐊"
            tooltipText: "Ask the phone to start camera sharing"
            foreground: root.foreground
            fontFamily: root.fontFamily
            enabled: root.canStart
            hasCursor: keyCatcher.activeFocus && root.actionCursor === 0
            Accessible.role: Accessible.Button
            Accessible.name: "Start camera sharing request"
            Accessible.description: "Requires approval on the trusted phone"
            Accessible.onPressAction: root.requestAction("start")
            onHovered: function(isHovered) { if (isHovered) root.actionCursor = 0 }
            onClicked: root.requestAction("start")
          }

          PanelActionButton {
            id: stopButton
            iconText: "󰓛"
            tooltipText: "Stop camera sharing"
            foreground: root.foreground
            fontFamily: root.fontFamily
            enabled: root.canStop
            hasCursor: keyCatcher.activeFocus && root.actionCursor === 1
            Accessible.role: Accessible.Button
            Accessible.name: "Stop camera sharing"
            Accessible.description: "Invalidates capture before cleanup and clears old camera frames"
            Accessible.onPressAction: root.requestAction("stop")
            onHovered: function(isHovered) { if (isHovered) root.actionCursor = 1 }
            onClicked: root.requestAction("stop")
          }

          PanelActionButton {
            id: forgetButton
            iconText: root.forgetArmed ? "󰗨" : "󰆴"
            tooltipText: root.forgetArmed ? "Press again to forget this phone" : "Forget trusted phone"
            foreground: root.foreground
            hoverColor: root.bar ? root.bar.urgent : Color.urgent
            fontFamily: root.fontFamily
            enabled: root.canForget
            hasCursor: keyCatcher.activeFocus && root.actionCursor === 2
            Accessible.role: Accessible.Button
            Accessible.name: root.forgetArmed ? "Confirm forgetting trusted phone" : "Forget trusted phone"
            Accessible.description: "Requires a second activation; revokes desktop trust and stops sharing"
            Accessible.onPressAction: root.armOrForget()
            onHovered: function(isHovered) { if (isHovered) root.actionCursor = 2 }
            onClicked: root.armOrForget()
          }

          Text {
            width: parent.width - startButton.width - stopButton.width - forgetButton.width - parent.spacing * 3
            anchors.verticalCenter: parent.verticalCenter
            text: root.forgetArmed ? "Forget removes desktop trust and stops sharing. Press again to confirm."
              : root.stateSummary()
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
            Accessible.role: Accessible.StatusBar
            Accessible.name: root.stateSummary()
            Accessible.description: "Current OmaCam trust, connection, capture, and output status"
          }
        }

        Text {
          width: parent.width
          text: root.onboardingText()
          textFormat: Text.PlainText
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        Row {
          width: parent.width
          spacing: Style.space(10)

          PanelActionButton {
            iconText: "󰑐"
            tooltipText: "Refresh service state (R)"
            foreground: root.foreground
            fontFamily: root.fontFamily
            enabled: !root.checking
            hasCursor: keyCatcher.activeFocus && root.actionCursor === 3
            Accessible.role: Accessible.Button
            Accessible.name: "Refresh OmaCam state"
            Accessible.description: "Reads a bounded service snapshot without changing system settings"
            Accessible.onPressAction: root.refresh()
            onHovered: function(isHovered) { if (isHovered) root.actionCursor = 3 }
            onClicked: root.refresh()
          }

          PanelActionButton {
            iconText: root.diagnosticsExpanded ? "󰅀" : "󰋼"
            tooltipText: "Show or hide read-only diagnostics (D)"
            foreground: root.foreground
            fontFamily: root.fontFamily
            enabled: !diagnosticsRequest.running
            hasCursor: keyCatcher.activeFocus && root.actionCursor === 4
            Accessible.role: Accessible.Button
            Accessible.name: root.diagnosticsExpanded ? "Hide diagnostics" : "Show diagnostics"
            Accessible.description: "Runs read-only diagnostics without exporting camera frames or secrets"
            Accessible.onPressAction: root.toggleDiagnostics()
            onHovered: function(isHovered) { if (isHovered) root.actionCursor = 4 }
            onClicked: root.toggleDiagnostics()
          }

          Text {
            anchors.verticalCenter: parent.verticalCenter
            text: diagnosticsRequest.running ? "Running read-only diagnostics…"
              : "Arrow keys select actions; Enter or Space activates. R refreshes, D shows diagnostics."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
            width: parent.width - Style.space(70)
          }
        }

        Text {
          width: parent.width
          visible: root.diagnosticsExpanded
          text: root.diagnosticsError !== "" ? root.diagnosticsError
            : (root.diagnosticsReport !== "" ? root.diagnosticsReport : "No diagnostics returned yet.")
          textFormat: Text.PlainText
          color: root.diagnosticsError !== "" ? (root.bar ? root.bar.urgent : Color.urgent) : root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WrapAnywhere
          Accessible.role: Accessible.StaticText
          Accessible.name: text
        }

        Text {
          width: parent.width
          visible: root.previewError !== ""
          text: root.previewError
          textFormat: Text.PlainText
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
          Accessible.role: Accessible.AlertMessage
          Accessible.name: text
        }

        Text {
          width: parent.width
          text: root.actionError !== "" ? root.actionError
            : (root.operationErrorText() !== ""
              ? root.operationErrorText()
              : (root.checkError !== "" ? root.checkError : root.report))
          textFormat: Text.PlainText
          color: (root.checkError !== "" || root.actionError !== "" || (root.snapshot && root.snapshot.last_error))
            ? (root.bar ? root.bar.urgent : Color.urgent) : root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WrapAnywhere
          Accessible.role: Accessible.StaticText
          Accessible.name: text
        }
      }
    }
  }
}
}
