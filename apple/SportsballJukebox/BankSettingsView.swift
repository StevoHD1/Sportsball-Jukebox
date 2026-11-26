import SwiftUI

/// Settings for the bank (page) shown: Multiplay, and its own settings
/// for new pads (overriding the show's), optionally applied to the pads
/// already on it.
struct BankSettingsView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var bank = 0
    @State private var multiplay = true
    @State private var ownDefaults = false
    @State private var defaults = PadDefaults()
    @State private var applyToPads = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Bank Settings: \(bankName)").font(.title2.bold())

            Toggle("Multiplay", isOn: $multiplay)
                .toggleStyle(.switch)
            Text("On: this bank's pads play over each other. Off: firing a pad fades out the bank's other pads.")
                .font(.caption)
                .foregroundStyle(.secondary)

            Divider()

            Toggle("Own settings for new pads", isOn: $ownDefaults)
            if ownDefaults {
                PadDefaultsEditor(defaults: $defaults, outputs: model.show.outputs)
            } else {
                Text("New pads on this bank use the show's settings (Show Settings → New pads).")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Toggle("Also apply to this bank's existing pads", isOn: $applyToPads)
            Text("Gives the pads already on this bank these colours, fades, loop, group, output, ducking and level matching when you click Apply. Undo reverses it.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Apply") {
                    apply()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 540)
        .onAppear {
            bank = model.currentBank
            multiplay = model.isMultiplay(bank: bank)
            ownDefaults = model.show.hasOwnDefaults(bank: bank)
            defaults = model.show.defaults(forBank: bank)
        }
    }

    private var bankName: String {
        model.show.banks.indices.contains(bank) ? model.show.banks[bank] : "Bank \(bank + 1)"
    }

    private func apply() {
        if multiplay != model.isMultiplay(bank: bank) {
            model.setMultiplay(bank: bank, multiplay)
        }
        let wanted: PadDefaults? = ownDefaults ? defaults : nil
        let current: PadDefaults? = model.show.hasOwnDefaults(bank: bank) ? model.show.defaults(forBank: bank) : nil
        if wanted != current {
            model.setBankDefaults(bank: bank, wanted)
        }
        if applyToPads {
            model.applyPadDefaults(bank: bank)
        }
    }
}
