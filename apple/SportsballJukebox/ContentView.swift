import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            HeaderBar()
            Divider()
            OutputBanners()
            NoticeToasts()
            if model.show.banks.count > 1 {
                BankBar()
            }
            ScrollView {
                PadGrid()
                    .padding(16)
            }
            Divider()
            ControlFooter()
        }
        .frame(minWidth: 820, minHeight: 480)
        .navigationTitle(model.windowTitle)
        .overlay {
            if model.isLoadingShow {
                ProgressView("Loading show…")
                    .padding(24)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .alert(item: $model.alert) { alert in
            Alert(title: Text(alert.title), message: Text(alert.message))
        }
        .sheet(item: $model.editing) { target in
            CueEditorView(slot: target.id)
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingOutputs) {
            OutputsView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingSettings) {
            ShowSettingsView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingControls) {
            ControlsView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingPreflight) {
            PreflightView()
                .environmentObject(model)
        }
    }
}

private struct PadGrid: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let columns = Array(
            repeating: GridItem(.flexible(), spacing: 10),
            count: max(model.show.columns, 1)
        )
        LazyVGrid(columns: columns, spacing: 10) {
            ForEach(Array(model.show.padRange(bank: model.currentBank)), id: \.self) { slot in
                CueButton(slot: slot)
            }
        }
    }
}

private struct HeaderBar: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 14) {
            Button {
                model.showingOutputs = true
            } label: {
                Label("Outputs", systemImage: "hifispeaker.2")
            }
            .help("Choose audio interfaces and backups")

            ForEach(Array((model.status?.outputs ?? []).enumerated()), id: \.offset) { _, output in
                OutputBadge(output: output)
            }

            Spacer()

            Button {
                model.runPreflight()
                model.showingPreflight = true
            } label: {
                Label("Pre-game Check", systemImage: "checklist")
            }
            .help("Check devices, files, keys and MIDI before the game")

            Button {
                model.showingControls = true
            } label: {
                Label("MIDI & OSC", systemImage: "pianokeys")
            }
            .help("External control: MIDI controllers, OSC, Stream Deck")

            Image(systemName: "speaker.wave.2.fill")
                .foregroundStyle(.secondary)
            Slider(value: $model.masterVolume, in: 0...1)
                .frame(width: 160)
                .help("Master volume")

            Button(role: .destructive) {
                model.stopAll()
            } label: {
                Label("Stop All", systemImage: "stop.fill")
                    .font(.headline)
                    .padding(.horizontal, 6)
            }
            .buttonStyle(.borderedProminent)
            .tint(.red)
            .keyboardShortcut(.escape, modifiers: [])
            .help("Fade out everything (Esc)")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
    }
}

private struct OutputBadge: View {
    let output: OutputState

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 9, height: 9)
            Text(text)
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .help(output.error ?? (output.deviceName ?? ""))
    }

    private var color: Color {
        switch output.status {
        case "running": return output.xruns > 0 ? .orange : .green
        case "on_backup": return .orange
        default: return .red
        }
    }

    private var text: String {
        guard output.isUp else { return "\(output.name): offline" }
        var text = "\(output.name): \(output.deviceName ?? "?")"
        if output.status == "on_backup" {
            text += " (backup)"
        }
        if output.xruns > 0 {
            text += " · \(output.xruns) glitch\(output.xruns == 1 ? "" : "es")"
        }
        return text
    }
}

/// Persistent red banners for outputs with no device.
private struct OutputBanners: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let down = (model.status?.outputs ?? []).filter { !$0.isUp }
        ForEach(Array(down.enumerated()), id: \.offset) { _, output in
            Label(
                "Output \"\(output.name)\" has no audio device. Its pads cannot play. Retrying…",
                systemImage: "exclamationmark.triangle.fill"
            )
            .font(.headline)
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity)
            .padding(8)
            .background(Color.red)
        }
    }
}

/// Short-lived messages such as "switched to backup device".
private struct NoticeToasts: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ForEach(Array(model.notices.enumerated()), id: \.offset) { _, notice in
            Label(notice, systemImage: "info.circle.fill")
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                .background(Color.orange.opacity(0.25))
        }
    }
}

/// Tabs for the show's banks of pads.
private struct BankBar: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 6) {
            ForEach(Array(model.show.banks.enumerated()), id: \.offset) { index, name in
                Button {
                    model.selectBank(index)
                } label: {
                    Text(index < 9 ? "\(name)  ⌘\(index + 1)" : name)
                        .font(.callout.weight(index == model.currentBank ? .semibold : .regular))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 5)
                        .background(
                            index == model.currentBank ? Color.accentColor : Color.secondary.opacity(0.15),
                            in: Capsule()
                        )
                        .foregroundStyle(index == model.currentBank ? Color.white : Color.primary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Bank \(name)")
                .accessibilityAddTraits(index == model.currentBank ? .isSelected : [])
            }
            Spacer()
            Text("Pad keys apply to the bank shown.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }
}

/// Bottom line: MIDI / OSC state and the last control message received.
private struct ControlFooter: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 14) {
            if let learning = model.learningTarget {
                Label("Waiting for MIDI to learn \(describe(learning))…", systemImage: "dot.radiowaves.left.and.right")
                    .foregroundStyle(.orange)
                Button("Cancel") { model.cancelLearn() }
            } else if let last = model.status?.lastControl {
                Label(last, systemImage: "dot.radiowaves.left.and.right")
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Text(midiText)
            Text(oscText)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(.bar)
    }

    private var midiText: String {
        guard let inputs = model.status?.midiInputs else { return "MIDI off" }
        switch inputs.count {
        case 0: return "MIDI: no devices"
        case 1: return "MIDI: \(inputs[0])"
        default: return "MIDI: \(inputs.count) devices"
        }
    }

    private var oscText: String {
        model.status?.oscPort.map { "OSC: port \($0)" } ?? "OSC off"
    }

    private func describe(_ target: String) -> String {
        switch target {
        case "stop_all": return "Stop All"
        case "next_bank": return "Next Bank"
        case "prev_bank": return "Previous Bank"
        default:
            let slot = Int(target.dropFirst(4)) ?? 0
            return model.show.cue(at: slot).map { "\"\($0.title)\"" } ?? "pad"
        }
    }
}
