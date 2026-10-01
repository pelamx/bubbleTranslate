import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Written out by `bubbleTranslate --omarchy-install`, which fills in the path
// of the binary below. Everything it shows comes from `--status`; a click is
// `--toggle`, which the running copy acts on.
BarWidget {
  id: root
  moduleName: "app.bubbletranslate"

  readonly property string binary: "@BUBBLETRANSLATE@"

  property bool running: false
  property bool auto: true
  property var remaining: null

  function refresh() {
    if (!statusProc.running) statusProc.running = true
  }

  function apply(text) {
    try {
      var s = JSON.parse(text)
      root.running = s.running === true
      root.auto = s.auto !== false
      root.remaining = (typeof s.remaining === "number") ? s.remaining : null
    } catch (e) {
      root.running = false
    }
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  Process {
    id: statusProc
    command: [root.binary, "--status"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.apply(text)
    }
  }

  Process {
    id: toggleProc
    command: [root.binary, "--toggle"]
    // `--toggle` returns once the request is sent, not once it is acted on,
    // so the state is read back a moment later rather than at once.
    onExited: settle.restart()
  }

  Timer {
    id: settle
    interval: 400
    onTriggered: root.refresh()
  }

  Timer {
    interval: 5000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // Material Design's translate and translate-off.
    text: root.running && !root.auto ? "\u{F0E06}" : "\u{F05CA}"
    dimmed: !root.running || !root.auto
    tooltipText: {
      if (!root.running) return "bubbleTranslate is not running"
      var line = root.auto ? "bubbleTranslate: bubbles on" : "bubbleTranslate: bubbles off"
      if (root.remaining !== null) line += " · " + root.remaining + " free left today"
      return line + "\nClick to turn " + (root.auto ? "off" : "on")
    }
    onPressed: function(b) {
      if (!root.running) return
      root.auto = !root.auto
      if (!toggleProc.running) toggleProc.running = true
    }
  }
}
