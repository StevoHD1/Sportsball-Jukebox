import SwiftUI

/// The show's colour presets: name, colour, order. Changing a preset's
/// colour can recolour the pads that use it.
struct ColourPresetsView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var rows: [Row] = []
    @State private var recolor = true

    struct Row: Identifiable, Equatable {
        let id = UUID()
        var name: String
        var color: Color
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Colour Presets").font(.title2.bold())
            Text("Saved with the show, so they're there on any Mac. They appear at the top of a pad's Colour menu.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            List {
                ForEach($rows) { $row in
                    HStack {
                        ColorPicker("Colour", selection: $row.color, supportsOpacity: false)
                            .labelsHidden()
                        TextField("Name", text: $row.name)
                            .textFieldStyle(.roundedBorder)
                        Text("\(padCount(row)) pad\(padCount(row) == 1 ? "" : "s")")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .frame(width: 56, alignment: .trailing)
                        Button(role: .destructive) {
                            rows.removeAll { $0.id == row.id }
                        } label: {
                            Image(systemName: "minus.circle.fill")
                        }
                        .buttonStyle(.borderless)
                        .help("Remove the preset (pads keep their colour)")
                    }
                }
                .onMove { rows.move(fromOffsets: $0, toOffset: $1) }
            }
            .frame(minHeight: 200)
            HStack {
                Button {
                    rows.append(Row(name: "Colour \(rows.count + 1)", color: .blue))
                } label: {
                    Label("Add Preset", systemImage: "plus")
                }
                Spacer()
                Toggle("Recolour pads when a preset's colour changes", isOn: $recolor)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    model.setShowColors(
                        rows.map { ShowColor(name: $0.name, value: PadColor.hex($0.color) ?? "#3478F6") },
                        recolor: recolor
                    )
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 520, height: 420)
        .onAppear {
            rows = model.show.colors.map { Row(name: $0.name, color: PadColor.color(named: $0.value)) }
        }
    }

    /// Pads using this preset's saved colour (before any edit).
    private func padCount(_ row: Row) -> Int {
        guard let saved = model.show.colors.first(where: { $0.name == row.name }) else { return 0 }
        return model.show.cues.filter {
            $0.color?.caseInsensitiveCompare(saved.value) == .orderedSame
        }.count
    }
}
