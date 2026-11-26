import SwiftUI

/// Edit the show's logical outputs: a name, a preferred device and a
/// backup used automatically if the preferred one fails.
struct OutputsView: View {
    /// An output being edited, with a stable identity so rows can be
    /// removed safely.
    private struct Row: Identifiable {
        let id = UUID()
        var doc: OutputDoc
    }

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var rows: [Row] = []

    private var outputs: [OutputDoc] { rows.map(\.doc) }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Audio Outputs").font(.title2.bold())
                Spacer()
                Button {
                    model.refreshDevices()
                } label: {
                    Label("Rescan Devices", systemImage: "arrow.clockwise")
                }
            }
            Text("Each pad plays through one output. Outputs run independently, so each can use a different interface. If an output's device fails, it switches to the backup (or the system default) and carries on from where it was.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            ScrollView {
                VStack(spacing: 10) {
                    ForEach($rows) { $row in
                        OutputRow(
                            output: $row.doc,
                            state: state(for: row.id),
                            devices: model.devices,
                            canRemove: rows.count > 1,
                            onRemove: { rows.removeAll { $0.id == row.id } }
                        )
                    }
                }
            }
            .frame(minHeight: 200)

            HStack {
                Text("Preview (headphones) output")
                Picker("", selection: previewOutput) {
                    Text("Off").tag(-1)
                    ForEach(Array(model.show.outputs.enumerated()), id: \.offset) { index, output in
                        Text(output.name).tag(index)
                    }
                }
                .frame(width: 200)
                Text("⌥-click a pad to hear it there only.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            HStack {
                Button {
                    rows.append(Row(doc: OutputDoc(name: "Output \(rows.count + 1)")))
                } label: {
                    Label("Add Output", systemImage: "plus")
                }
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Apply") {
                    model.applyOutputs(outputs)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(outputs == model.show.outputs)
            }
        }
        .padding(20)
        .frame(width: 640, height: 480)
        .onAppear {
            model.refreshDevices()
            rows = model.show.outputs.map { Row(doc: $0) }
        }
    }

    /// Applies immediately (preview is not part of the output list edits).
    private var previewOutput: Binding<Int> {
        Binding(
            get: { model.show.previewOutput ?? -1 },
            set: { model.setPreviewOutput($0 < 0 ? nil : $0) }
        )
    }

    /// Live status for a row that already exists in the show.
    private func state(for id: UUID) -> OutputState? {
        guard let index = rows.firstIndex(where: { $0.id == id }),
              index < model.show.outputs.count,
              rows[index].doc == model.show.outputs[index]
        else { return nil }
        let states = model.status?.outputs ?? []
        return states.indices.contains(index) ? states[index] : nil
    }
}

private struct OutputRow: View {
    @Binding var output: OutputDoc
    let state: OutputState?
    let devices: [AudioDevice]
    let canRemove: Bool
    let onRemove: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                TextField("Name", text: $output.name)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 220)
                Spacer()
                if let state {
                    Text(statusText(state))
                        .font(.caption)
                        .foregroundStyle(state.isUp ? Color.secondary : Color.red)
                }
                Button(role: .destructive, action: onRemove) {
                    Image(systemName: "trash")
                }
                .disabled(!canRemove)
                .help("Remove output (its pads move to the first output)")
            }
            DevicePicker(title: "Device", selection: $output.device, devices: devices,
                         noneLabel: "System Default")
            DevicePicker(title: "Backup", selection: $output.backup, devices: devices,
                         noneLabel: "None (use System Default)")
        }
        .padding(12)
        .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
    }

    private func statusText(_ state: OutputState) -> String {
        switch state.status {
        case "running":
            return "Playing on \(state.deviceName ?? "?") · \(state.sampleRate / 1000) kHz"
        case "on_backup":
            return "On backup: \(state.deviceName ?? "?")"
        default:
            return state.error ?? "Offline"
        }
    }
}

/// Picks a device by id; remembers its name too, so a show can find the
/// same model of interface on another Mac.
private struct DevicePicker: View {
    let title: String
    @Binding var selection: DeviceRef?
    let devices: [AudioDevice]
    let noneLabel: String

    var body: some View {
        Picker(title, selection: idBinding) {
            Text(noneLabel).tag("")
            ForEach(devices) { device in
                Text(device.name).tag(device.id)
            }
            // Keep a remembered device selectable even when unplugged.
            if let selection, connected == nil {
                Text("\(selection.name.isEmpty ? selection.id : selection.name) (not connected)")
                    .tag(selection.id)
            }
        }
    }

    /// The connected device the selection refers to: by id, or by name
    /// (as the engine also matches, e.g. after moving to another Mac).
    private var connected: AudioDevice? {
        guard let selection else { return nil }
        return devices.first { $0.id == selection.id }
            ?? devices.first { !selection.name.isEmpty && $0.name == selection.name }
    }

    private var idBinding: Binding<String> {
        Binding(
            get: { connected?.id ?? selection?.id ?? "" },
            set: { id in
                if id.isEmpty {
                    selection = nil
                } else if let device = devices.first(where: { $0.id == id }) {
                    selection = DeviceRef(id: device.id, name: device.name)
                }
            }
        )
    }
}

/// Show name, grid size and banks.
struct ShowSettingsView: View {
    private struct Bank: Identifiable {
        let id = UUID()
        var name: String
    }

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var slots = 24
    @State private var columns = 6
    @State private var banks: [Bank] = []
    @State private var stopAllFade = 0.5
    @State private var levelMatchOn = false
    @State private var levelTarget = -16.0
    @State private var padDefaults = PadDefaults()
    @State private var applyToPads = false
    @State private var playedOn = false
    @State private var playedColor = "#555555"
    @State private var markPlayed = true

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Show Settings").font(.title2.bold())
            Form {
                TextField("Name", text: $name)
                Stepper("Pads per bank: \(slots)", value: $slots, in: 1...128)
                Stepper("Columns: \(columns)", value: $columns, in: 1...16)
                HStack {
                    Text("Stop All fade")
                    TextField("", value: $stopAllFade, format: .number.precision(.fractionLength(0...2)))
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 60)
                    Text("seconds").foregroundStyle(.secondary)
                }
                Toggle("Level-match pads", isOn: $levelMatchOn)
                    .help("Play every pad and playlist song at the same loudness")
                if levelMatchOn {
                    Picker("Target", selection: $levelTarget) {
                        Text("-23 LUFS (broadcast, quiet)").tag(-23.0)
                        Text("-18 LUFS").tag(-18.0)
                        Text("-16 LUFS (recommended)").tag(-16.0)
                        Text("-14 LUFS (streaming, loud)").tag(-14.0)
                        if ![-23.0, -18.0, -16.0, -14.0].contains(levelTarget) {
                            Text(String(format: "%.1f LUFS (from the show file)", levelTarget)).tag(levelTarget)
                        }
                    }
                }
            }
            if levelMatchOn {
                Text("Each file is measured when loaded and turned up or down to the target, so a quiet walk-up and a loud horn play at the same level. Quiet files are boosted by at most 12 dB and never past -1 dBFS. A pad can opt out in its editor. Changing this stops playing pads.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            DisclosureGroup("Played pads") {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Toggle("Show played pads in", isOn: $playedOn)
                        PlayedColourPicker(value: $playedColor)
                            .disabled(!playedOn)
                    }
                    Toggle("Mark pads as played when they fire", isOn: $markPlayed)
                        .disabled(!playedOn)
                    Text("Played pads show a ✓ (and this colour), so you can see what's been used this game. Right-click a pad to mark or unmark it; Reset Played on the bank bar clears the page.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.top, 6)
            }
            DisclosureGroup("New pads") {
                VStack(alignment: .leading, spacing: 8) {
                    PadDefaultsEditor(defaults: $padDefaults, outputs: model.show.outputs)
                    Toggle("Also apply to every existing pad", isOn: $applyToPads)
                    Text("Gives existing pads these colours, fades, loop, group, output, ducking and level matching when you click Apply. Banks with their own settings (Bank Settings…) get theirs. Undo reverses it.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.top, 6)
            }
            Text("Banks").font(.headline)
            Text("Each bank is a page of pads. Switch with ⌘1–⌘9, the bank tabs, MIDI or OSC.")
                .font(.callout)
                .foregroundStyle(.secondary)
            ForEach($banks) { $bank in
                HStack {
                    TextField("Bank name", text: $bank.name)
                        .textFieldStyle(.roundedBorder)
                    Button(role: .destructive) {
                        banks.removeAll { $0.id == bank.id }
                    } label: {
                        Image(systemName: "trash")
                    }
                    .disabled(banks.count <= 1)
                }
            }
            Button {
                banks.append(Bank(name: "Bank \(banks.count + 1)"))
            } label: {
                Label("Add Bank", systemImage: "plus")
            }
            .disabled(slots * (banks.count + 1) > 512)
            Text("Total: \(slots * banks.count) pads (max 512).")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Apply") {
                    model.applySettings(
                        name: name,
                        slots: slots,
                        columns: columns,
                        banks: banks.map(\.name),
                        stopAllFade: stopAllFade,
                        levelMatch: levelMatchOn ? levelTarget : nil,
                        playedColor: playedOn ? playedColor : nil,
                        markPlayed: markPlayed
                    )
                    if padDefaults != (model.show.padDefaults ?? PadDefaults()) {
                        model.setPadDefaults(padDefaults == PadDefaults() ? nil : padDefaults)
                    }
                    if applyToPads {
                        model.applyPadDefaults(bank: nil)
                    }
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(slots * banks.count > 512)
            }
        }
        .padding(20)
        .frame(width: 540)
        .onAppear {
            name = model.show.name
            slots = model.show.slots
            columns = model.show.columns
            banks = model.show.banks.map { Bank(name: $0) }
            stopAllFade = model.show.stopAllFade
            levelMatchOn = model.show.levelMatch != nil
            if let target = model.show.levelMatch {
                levelTarget = target
            }
            padDefaults = model.show.padDefaults ?? PadDefaults()
            playedOn = model.show.playedColor != nil
            playedColor = model.show.playedColor ?? "#555555"
            markPlayed = model.show.markPlayed
        }
    }
}

/// What a new pad starts with when audio is dropped on an empty pad.
struct PadDefaultsEditor: View {
    @EnvironmentObject private var model: AppModel
    @Binding var defaults: PadDefaults
    let outputs: [OutputDoc]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Colour")
                Picker("Colour", selection: color) {
                    Text("Default").tag("")
                    ForEach(model.show.colors, id: \.self) { preset in
                        Text(preset.name).tag(preset.value)
                    }
                    ForEach(PadColor.allCases, id: \.self) { c in
                        Text(c.rawValue.capitalized).tag(c.rawValue)
                    }
                    if let custom = defaults.color, PadColor(rawValue: custom) == nil,
                       model.show.preset(for: custom) == nil {
                        Text("Custom").tag(custom)
                    }
                }
                .labelsHidden()
                .frame(width: 120)
                ColorPicker("Custom colour", selection: customColor, supportsOpacity: false)
                    .labelsHidden()
                Spacer()
                Picker("Output", selection: $defaults.output) {
                    ForEach(Array(outputs.enumerated()), id: \.offset) { index, output in
                        Text(output.name).tag(index)
                    }
                }
                .frame(width: 170)
            }
            HStack {
                Text("Fade in").fixedSize()
                TextField("", value: $defaults.fadeIn, format: .number.precision(.fractionLength(0...2)))
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 50)
                Text("out").fixedSize()
                TextField("", value: fadeOut, format: .number.precision(.fractionLength(0...2)))
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 50)
                Text("s").foregroundStyle(.secondary)
                Spacer()
                Text("Group").fixedSize()
                TextField("none", text: group)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 110)
            }
            HStack {
                Toggle("Loop", isOn: $defaults.loop)
                Toggle("Duck others", isOn: ducking)
                if ducking.wrappedValue {
                    Slider(value: duckDB, in: -40 ... -3).frame(width: 80)
                    Text(String(format: "%.0f dB", duckDB.wrappedValue))
                        .font(.caption.monospacedDigit())
                }
            }
            HStack {
                Toggle("Skip silence at the start", isOn: $defaults.trimSilence)
                Toggle("Level-match", isOn: $defaults.levelMatch)
                Spacer()
                Button("Reset") { defaults = PadDefaults() }
                    .disabled(defaults == PadDefaults())
            }
            Text("Applied when audio is dropped on an empty pad. Right-click a pad → Use Style for New Pads to copy its settings here.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var color: Binding<String> {
        Binding(
            get: { defaults.color ?? "" },
            set: { defaults.color = $0.isEmpty ? nil : $0 }
        )
    }

    private var customColor: Binding<Color> {
        Binding(
            get: { PadColor.color(named: defaults.color ?? PadColor.blue.rawValue) },
            set: { if let hex = PadColor.hex($0) { defaults.color = hex } }
        )
    }

    private var fadeOut: Binding<Double> {
        Binding(
            get: { defaults.fadeOut ?? 0.5 },
            set: { defaults.fadeOut = $0 == 0.5 ? nil : max($0, 0) }
        )
    }

    private var group: Binding<String> {
        Binding(
            get: { defaults.group ?? "" },
            set: {
                let trimmed = $0.trimmingCharacters(in: .whitespaces)
                defaults.group = trimmed.isEmpty ? nil : trimmed
            }
        )
    }

    private var ducking: Binding<Bool> {
        Binding(
            get: { defaults.duck != nil },
            set: { defaults.duck = $0 ? (defaults.duck ?? -12) : nil }
        )
    }

    private var duckDB: Binding<Double> {
        Binding(
            get: { Double(defaults.duck ?? -12) },
            set: { defaults.duck = Float($0.rounded()) }
        )
    }
}

/// A colour for played pads: show presets, standard colours, or any.
private struct PlayedColourPicker: View {
    @EnvironmentObject private var model: AppModel
    @Binding var value: String

    var body: some View {
        HStack(spacing: 6) {
            Menu {
                if !model.show.colors.isEmpty {
                    Section("Show colours") {
                        ForEach(model.show.colors, id: \.self) { preset in
                            Button(preset.name) { value = preset.value }
                        }
                    }
                }
                Section("Standard") {
                    Button("Dark grey") { value = "#555555" }
                    ForEach(PadColor.allCases, id: \.self) { preset in
                        Button(preset.rawValue.capitalized) { value = preset.rawValue }
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    Image(nsImage: PadColor.swatch(PadColor.color(named: value)))
                    Text(model.show.preset(for: value)?.name ?? (PadColor(rawValue: value)?.rawValue.capitalized ?? value))
                }
            }
            .fixedSize()
            ColorPicker("Played colour", selection: Binding(
                get: { PadColor.color(named: value) },
                set: { if let hex = PadColor.hex($0) { value = hex } }
            ), supportsOpacity: false)
            .labelsHidden()
        }
    }
}
