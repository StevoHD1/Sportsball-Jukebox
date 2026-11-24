import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    private var columns: [GridItem] {
        Array(repeating: GridItem(.flexible(), spacing: 10), count: AppModel.columns)
    }

    var body: some View {
        VStack(spacing: 0) {
            HeaderBar()
            Divider()
            if let banner = outputProblem {
                Label(banner, systemImage: "exclamationmark.triangle.fill")
                    .font(.headline)
                    .foregroundStyle(.white)
                    .frame(maxWidth: .infinity)
                    .padding(8)
                    .background(Color.red)
            }
            ScrollView {
                LazyVGrid(columns: columns, spacing: 10) {
                    ForEach(model.cues) { cue in
                        CueButton(cue: cue)
                    }
                }
                .padding(16)
            }
        }
        .frame(minWidth: 820, minHeight: 480)
        .alert(item: $model.alert) { alert in
            Alert(title: Text(alert.title), message: Text(alert.message))
        }
    }

    private var outputProblem: String? {
        if model.isOpeningOutput { return nil }
        switch model.output?.health {
        case .none:
            return "No audio output. Choose a device above."
        case .some(.deviceLost):
            return "Audio device disconnected. Choose another output above."
        case .some(.failed):
            return "Audio output failed. Re-select the device above."
        case .some(.running):
            return nil
        }
    }
}

private struct HeaderBar: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 14) {
            Picker("Output", selection: deviceSelection) {
                Text("System Default").tag("")
                ForEach(model.devices) { device in
                    Text(device.name).tag(device.id)
                }
            }
            .frame(maxWidth: 340)
            .disabled(model.isOpeningOutput)

            Button {
                model.refreshDevices()
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .help("Rescan audio devices")

            OutputStatus()

            Spacer()

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

    private var deviceSelection: Binding<String> {
        Binding(
            get: { model.selectedDeviceID ?? "" },
            set: { model.selectDevice($0.isEmpty ? nil : $0) }
        )
    }
}

private struct OutputStatus: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 9, height: 9)
            Text(text)
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
        }
        .help("Audio output status")
    }

    private var color: Color {
        if model.isOpeningOutput { return .yellow }
        switch model.output?.health {
        case .some(.running): return (model.output?.xruns ?? 0) > 0 ? .orange : .green
        default: return .red
        }
    }

    private var text: String {
        if model.isOpeningOutput { return "Opening…" }
        guard let output = model.output else { return "No output" }
        var text = String(format: "%.1f kHz · %d ch", Double(output.sampleRate) / 1000, output.channels)
        if output.xruns > 0 {
            text += " · \(output.xruns) glitch\(output.xruns == 1 ? "" : "es")"
        }
        return text
    }
}
