import SwiftUI

/// MIDI and OSC settings, show-wide MIDI controls, and the OSC cheat sheet.
struct ControlsView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var oscEnabled = false
    @State private var portText = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("MIDI & OSC").font(.title2.bold())

            GroupBox("MIDI") {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Toggle("Listen to MIDI devices", isOn: $model.midiEnabled)
                        Spacer()
                        Button("Rescan") { model.rescanMIDI() }
                            .disabled(!model.midiEnabled)
                    }
                    Text(inputsText)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    Text("To trigger a pad from MIDI, right-click it and choose Learn MIDI Trigger, then press a key, pad or button on your controller.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Divider()
                    ControlRow(title: "Stop All", target: "stop_all", trigger: model.show.controls.stopAll)
                    ControlRow(title: "Next Bank", target: "next_bank", trigger: model.show.controls.nextBank)
                    ControlRow(title: "Previous Bank", target: "prev_bank", trigger: model.show.controls.prevBank)
                }
                .padding(6)
            }

            GroupBox("OSC (network control, Stream Deck, TouchOSC, QLab)") {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Toggle("Listen for OSC on UDP port", isOn: $oscEnabled)
                        TextField("53100", text: $portText)
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 80)
                            .disabled(!oscEnabled)
                        Button("Apply") { applyOSC() }
                        Spacer()
                        if let port = model.status?.oscPort {
                            Label("Listening on \(String(port))", systemImage: "checkmark.circle.fill")
                                .foregroundStyle(.green)
                        }
                    }
                    Text(oscHelp)
                        .font(.system(.callout, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                    Text("Stream Deck: use an OSC or MIDI plugin and point it at this Mac's port, or use the Hotkey action with a pad's key while this app is in front.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(6)
            }

            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 640)
        .onAppear {
            oscEnabled = model.oscPort != 0
            portText = String(model.oscPort != 0 ? model.oscPort : 53100)
        }
    }

    private var inputsText: String {
        guard let inputs = model.status?.midiInputs else { return "MIDI input is off." }
        if inputs.isEmpty { return "No MIDI devices found. Connect one and press Rescan." }
        return "Listening to: " + inputs.joined(separator: ", ")
    }

    private var oscHelp: String {
        """
        /sbj/fire/<pad>         fire a pad in the bank shown
        /sbj/fire/<bank>/<pad>  fire a pad in a given bank
        /sbj/title  "Goal Horn" fire the pad with that title
        /sbj/stop/<pad>   /sbj/fade/<pad>
        /sbj/stopall   /sbj/stopall/now
        /sbj/bank/<n>   /sbj/bank/next   /sbj/bank/prev
        """
    }

    private func applyOSC() {
        let port = Int(portText.trimmingCharacters(in: .whitespaces)) ?? 0
        model.oscPort = oscEnabled ? min(max(port, 1024), 65535) : 0
    }
}

/// A show-wide action with its MIDI trigger, Learn and Clear.
private struct ControlRow: View {
    @EnvironmentObject private var model: AppModel
    let title: String
    let target: String
    let trigger: MidiTrigger?

    var body: some View {
        HStack {
            Text(title).frame(width: 120, alignment: .leading)
            if model.learningTarget == target {
                Text("Press a MIDI control…").foregroundStyle(.orange)
                Spacer()
                Button("Cancel") { model.cancelLearn() }
            } else {
                Text(trigger?.label ?? "Not assigned")
                    .foregroundStyle(trigger == nil ? Color.secondary : Color.primary)
                Spacer()
                Button("Learn") { model.learnMIDI(target) }
                Button("Clear") { model.clearMIDI(target) }
                    .disabled(trigger == nil)
            }
        }
    }
}

/// The pre-game check.
struct PreflightView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Image(systemName: ready ? "checkmark.seal.fill" : "exclamationmark.octagon.fill")
                    .font(.system(size: 34))
                    .foregroundStyle(ready ? .green : .red)
                VStack(alignment: .leading) {
                    Text(ready ? "Ready for the game" : "Not ready yet")
                        .font(.title2.bold())
                    Text(ready ? "No errors found. Review any warnings below." : "Fix the errors below before doors open.")
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button {
                    model.runPreflight()
                } label: {
                    Label("Check Again", systemImage: "arrow.clockwise")
                }
            }
            List(model.preflight?.checks ?? []) { check in
                Label {
                    Text(check.message)
                        .fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: icon(check.level))
                        .foregroundStyle(color(check.level))
                }
                .accessibilityLabel("\(check.level): \(check.message)")
            }
            .frame(minHeight: 260)
            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 640, height: 480)
    }

    private var ready: Bool { model.preflight?.ready ?? false }

    private func icon(_ level: String) -> String {
        switch level {
        case "error": return "xmark.octagon.fill"
        case "warning": return "exclamationmark.triangle.fill"
        case "info": return "info.circle.fill"
        default: return "checkmark.circle.fill"
        }
    }

    private func color(_ level: String) -> Color {
        switch level {
        case "error": return .red
        case "warning": return .orange
        case "info": return .blue
        default: return .green
        }
    }
}
