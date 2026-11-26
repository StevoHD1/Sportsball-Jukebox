import SwiftUI

/// Change chosen settings on every pad in the show or on one bank
/// ("fade out 2 s on every pad in this bank"). Only ticked settings
/// change; everything else on each pad stays as it is.
struct BulkEditView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    /// nil = the whole show.
    @State private var bank: Int?

    @State private var changeColour = false
    @State private var colour = ""  // "" = default
    @State private var changeVolume = false
    @State private var volumeDB = 0.0
    @State private var changeFadeIn = false
    @State private var fadeIn = 0.0
    @State private var changeFadeOut = false
    @State private var fadeOut = 0.5
    @State private var changeLoop = false
    @State private var loop = false
    @State private var changeGroup = false
    @State private var group = ""
    @State private var changeOutput = false
    @State private var output = 0
    @State private var changeDuck = false
    @State private var duckOn = false
    @State private var duckDB = -12.0
    @State private var changeLevelMatch = false
    @State private var levelMatch = true
    @State private var changeRetrigger = false
    @State private var retrigger = Retrigger.restart
    @State private var changePlayMode = false
    @State private var playMode = PlayMode.bank

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Bulk Edit Pads").font(.title2.bold())
            Picker("Change pads on", selection: $bank) {
                Text("The whole show (\(padCount(nil)) pads)").tag(Int?.none)
                ForEach(Array(model.show.banks.enumerated()), id: \.offset) { index, name in
                    Text("\(name) (\(padCount(index)) pads)").tag(Int?.some(index))
                }
            }
            .frame(width: 380)
            Text("Tick the settings to change. Everything else on each pad stays as it is.")
                .font(.callout)
                .foregroundStyle(.secondary)

            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                row("Colour", $changeColour) {
                    Picker("Colour", selection: $colour) {
                        Text("Default").tag("")
                        ForEach(model.show.colors, id: \.self) { preset in
                            Text(preset.name).tag(preset.value)
                        }
                        ForEach(PadColor.allCases, id: \.self) { c in
                            Text(c.rawValue.capitalized).tag(c.rawValue)
                        }
                        if !colour.isEmpty, PadColor(rawValue: colour) == nil, model.show.preset(for: colour) == nil {
                            Text("Custom").tag(colour)
                        }
                    }
                    .labelsHidden()
                    .frame(width: 140)
                    ColorPicker("Custom colour", selection: customColour, supportsOpacity: false)
                        .labelsHidden()
                }
                row("Volume", $changeVolume) {
                    Slider(value: $volumeDB, in: -30...6, step: 0.5).frame(width: 160)
                    Text(String(format: "%+.1f dB", volumeDB)).font(.callout.monospacedDigit())
                }
                row("Fade in", $changeFadeIn) {
                    seconds($fadeIn)
                }
                row("Fade out", $changeFadeOut) {
                    seconds($fadeOut)
                }
                row("Loop", $changeLoop) {
                    Toggle("Loop", isOn: $loop).labelsHidden()
                    Text(loop ? "On" : "Off").foregroundStyle(.secondary)
                }
                row("Group", $changeGroup) {
                    TextField("none", text: $group)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 140)
                    Text("Empty = no group").font(.caption).foregroundStyle(.secondary)
                }
                row("Output", $changeOutput) {
                    Picker("Output", selection: $output) {
                        ForEach(Array(model.show.outputs.enumerated()), id: \.offset) { index, o in
                            Text(o.name).tag(index)
                        }
                    }
                    .labelsHidden()
                    .frame(width: 160)
                }
                row("Duck others", $changeDuck) {
                    Toggle("Duck", isOn: $duckOn).labelsHidden()
                    if duckOn {
                        Slider(value: $duckDB, in: -40 ... -3, step: 1).frame(width: 110)
                        Text(String(format: "%.0f dB", duckDB)).font(.callout.monospacedDigit())
                    } else {
                        Text("Off").foregroundStyle(.secondary)
                    }
                }
                row("Level-match", $changeLevelMatch) {
                    Toggle("Level-match", isOn: $levelMatch).labelsHidden()
                    Text(levelMatch ? "On" : "Off").foregroundStyle(.secondary)
                }
                row("Press again", $changeRetrigger) {
                    Picker("Press again", selection: $retrigger) {
                        ForEach(Retrigger.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    .labelsHidden()
                    .frame(width: 160)
                }
                row("Play mode", $changePlayMode) {
                    Picker("Play mode", selection: $playMode) {
                        ForEach(PlayMode.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    .labelsHidden()
                    .frame(width: 260)
                }
            }

            HStack {
                Text("Undo reverses it.").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(applyTitle) {
                    model.bulkEdit(bank: bank, changes: changes)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(changes.isEmpty || padCount(bank) == 0)
            }
        }
        .padding(20)
        .frame(width: 560)
        .onAppear {
            bank = model.bulkEditBank
        }
    }

    @ViewBuilder
    private func row<Content: View>(
        _ label: String, _ on: Binding<Bool>, @ViewBuilder content: () -> Content
    ) -> some View {
        GridRow {
            Toggle(label, isOn: on)
                .frame(width: 130, alignment: .leading)
            HStack(spacing: 8) { content() }
                .disabled(!on.wrappedValue)
                .opacity(on.wrappedValue ? 1 : 0.5)
        }
    }

    private func seconds(_ value: Binding<Double>) -> some View {
        HStack(spacing: 4) {
            TextField("", value: value, format: .number.precision(.fractionLength(0...2)))
                .textFieldStyle(.roundedBorder)
                .frame(width: 60)
            Text("seconds").foregroundStyle(.secondary)
        }
    }

    private var customColour: Binding<Color> {
        Binding(
            get: { PadColor.color(named: colour.isEmpty ? PadColor.blue.rawValue : colour) },
            set: { if let hex = PadColor.hex($0) { colour = hex } }
        )
    }

    private func padCount(_ bank: Int?) -> Int {
        guard let bank else { return model.show.cues.count }
        let slots = max(model.show.slots, 1)
        return model.show.cues.filter { $0.slot / slots == bank }.count
    }

    private var applyTitle: String {
        let n = padCount(bank)
        return "Change \(n) Pad\(n == 1 ? "" : "s")"
    }

    /// The JSON the engine takes: only ticked settings; NSNull clears.
    private var changes: [String: Any] {
        var c: [String: Any] = [:]
        if changeColour { c["color"] = colour.isEmpty ? NSNull() : colour }
        if changeVolume { c["gain"] = pow(10, volumeDB / 20) }
        if changeFadeIn { c["fade_in"] = max(fadeIn, 0) }
        if changeFadeOut { c["fade_out"] = max(fadeOut, 0) }
        if changeLoop { c["loop"] = loop }
        if changeGroup {
            let trimmed = group.trimmingCharacters(in: .whitespaces)
            c["group"] = trimmed.isEmpty ? NSNull() : trimmed
        }
        if changeOutput { c["output"] = output }
        if changeDuck { c["duck"] = duckOn ? duckDB : NSNull() }
        if changeLevelMatch { c["level_match"] = levelMatch }
        if changeRetrigger { c["retrigger"] = retrigger.rawValue }
        if changePlayMode { c["play_mode"] = playMode.rawValue }
        return c
    }
}
