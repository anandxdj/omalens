import QtQuick
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
  readonly property var barIdentity: hostWidget || root
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property bool outputReady: report.indexOf("OmaCam output writer         READY") !== -1
  readonly property string statusText: checking
    ? "OmaCam — checking camera setup"
    : (outputReady ? "OmaCam — camera output ready" : "OmaCam — setup needed")

  function open() {
    openedFromHotkey = false
    root.controller.show()
    refresh()
  }

  function openFromHotkey() {
    openedFromHotkey = true
    root.controller.show()
    refresh()
  }

  function close() {
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
    if (doctor.running) return
    checkError = ""
    checking = true
    doctor.running = true
  }

  Process {
    id: doctor
    command: ["omacam-daemon", "doctor"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var next = String(text || "").trim()
        if (next !== "") root.report = next
      }
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.checkError = String(text || "").trim()
    }
    onExited: function(exitCode) {
      root.checking = false
      if (exitCode !== 0 && root.checkError === "")
        root.checkError = "The OmaCam service is not installed or could not run."
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
      onReturnRequested: root.refresh()
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: content
        width: parent.width
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

          Text {
            anchors.centerIn: parent
            width: parent.width - Style.space(64)
            text: root.checking ? "Checking camera setup…" : (root.outputReady ? "Camera output ready" : "Camera setup needed")
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.title
            font.bold: true
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

        Row {
          width: parent.width
          spacing: Style.space(10)

          Column {
            width: parent.width - refreshButton.width - parent.spacing
            spacing: Style.space(3)

            Text {
              text: "Host readiness"
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.subtitle
              font.bold: true
            }
            Text {
              text: "Read-only checks. OmaCam does not change your system here."
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
            }
          }

          PanelActionButton {
            id: refreshButton
            anchors.verticalCenter: parent.verticalCenter
            iconText: "󰑐"
            tooltipText: "Check again"
            foreground: root.foreground
            fontFamily: root.fontFamily
            enabled: !root.checking
            hasCursor: keyCatcher.activeFocus
            onClicked: root.refresh()
          }
        }

        Text {
          width: parent.width
          text: root.checkError !== "" ? root.checkError : root.report
          textFormat: Text.PlainText
          color: root.checkError !== "" ? (root.bar ? root.bar.urgent : Color.urgent) : root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WrapAnywhere
        }
      }
    }
  }
}
