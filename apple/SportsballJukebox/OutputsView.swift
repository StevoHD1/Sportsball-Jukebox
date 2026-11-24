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
            if let selection, !devices.contains(where: { $0.id == selection.id }) {
                Text("\(selection.name.isEmpty ? selection.id : selection.name) (not connected)")
                    .tag(selection.id)
            }
        }
    }

    private var idBinding: Binding<String> {
        Binding(
            get: { selection?.id ?? "" },
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

/// Show name and grid size.
struct ShowSettingsView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var slots = 24
    @State private var columns = 6

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Show Settings").font(.title2.bold())
            Form {
                TextField("Name", text: $name)
                Stepper("Pads: \(slots)", value: $slots, in: 1...128)
                Stepper("Columns: \(columns)", value: $columns, in: 1...16)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Apply") {
                    model.applySettings(name: name, slots: slots, columns: columns)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 380)
        .onAppear {
            name = model.show.name
            slots = model.show.slots
            columns = model.show.columns
        }
    }
}
