import SwiftUI

/// The game rundown: an ordered list of pads and notes, fired one at a
/// time with GO (or Space). The standby step is highlighted.
struct RundownView: View {
    @EnvironmentObject private var model: AppModel
    var width: Double = 270
    @State private var editing: RundownItem?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Rundown").font(.headline)
                Spacer()
                Button {
                    editing = RundownItem(id: 0)
                } label: {
                    Label("Add Note", systemImage: "text.badge.plus")
                }
                .labelStyle(.iconOnly)
                .help("Add a note, e.g. \"Timeout 1\" or \"Halftime\"")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)

            StandbyPanel()
                .padding(.horizontal, 12)
                .padding(.bottom, 8)

            Divider()

            if model.show.rundown.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "list.number")
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                    Text("Right-click a pad and choose Add to Rundown to build the game's running order, then fire it step by step with GO or Space.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                .padding(20)
                .frame(maxHeight: .infinity)
            } else {
                List {
                    ForEach(Array(model.show.rundown.enumerated()), id: \.element.id) { index, item in
                        RundownRow(index: index, item: item)
                            .contentShape(Rectangle())
                            .onTapGesture { model.setStandby(index) }
                            .contextMenu {
                                Button("Put on Standby") { model.setStandby(index) }
                                    .disabled(item.pad == nil)
                                Button(item.pad == nil ? "Edit Note…" : "Edit Label…") { editing = item }
                                Divider()
                                Button("Remove", role: .destructive) {
                                    model.removeRundownItems(at: IndexSet(integer: index))
                                }
                            }
                    }
                    .onMove { model.moveRundownItems(from: $0, to: $1) }
                    .onDelete { model.removeRundownItems(at: $0) }
                }
                .listStyle(.sidebar)
            }
        }
        .frame(width: width)
        .sheet(item: $editing) { item in
            RundownTextEditor(item: item) { edited in
                if edited.id == 0 {
                    model.addToRundown(pad: nil, text: edited.text)
                } else {
                    model.updateRundownItem(edited)
                }
            }
        }
    }
}

/// What GO fires next, the GO button and back/next.
private struct StandbyPanel: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(caption)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(title)
                .font(.title3.bold())
                .lineLimit(1)
                .foregroundStyle(standbyItem == nil ? .secondary : .primary)
            HStack(spacing: 8) {
                Button {
                    model.stepStandby(forward: false)
                } label: {
                    Image(systemName: "chevron.left")
                }
                .help("Previous step")
                Button {
                    model.stepStandby(forward: true)
                } label: {
                    Image(systemName: "chevron.right")
                }
                .help("Skip this step")
                Button {
                    model.rundownGo()
                } label: {
                    Text("GO")
                        .font(.title2.weight(.heavy))
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .tint(.green)
                .controlSize(.large)
                .keyboardShortcut(.space, modifiers: [])
                .disabled(standbyItem == nil)
                .help("Fire the standby step (Space)")
                .accessibilityLabel("GO: \(title)")
            }
        }
    }

    private var standbyItem: RundownItem? {
        guard let i = model.standby, model.show.rundown.indices.contains(i) else { return nil }
        return model.show.rundown[i]
    }

    private var title: String {
        if let item = standbyItem { return model.rundownLabel(item) }
        return model.show.rundown.isEmpty ? "No steps yet" : "Finished"
    }

    private var caption: String {
        let steps = model.show.rundown.filter { $0.pad != nil }
        guard let i = model.standby, !steps.isEmpty else { return "Standby" }
        let done = model.show.rundown.prefix(i).filter { $0.pad != nil }.count
        return "Standby · step \(done + 1) of \(steps.count)"
    }
}

private struct RundownRow: View {
    @EnvironmentObject private var model: AppModel
    let index: Int
    let item: RundownItem

    var body: some View {
        if item.pad == nil {
            Text(item.text.isEmpty ? "Note" : item.text)
                .font(.callout.weight(.semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
                .padding(.top, 6)
                .accessibilityAddTraits(.isHeader)
        } else {
            HStack(spacing: 8) {
                Image(systemName: icon)
                    .foregroundStyle(isStandby ? Color.green : Color.secondary)
                    .frame(width: 16)
                VStack(alignment: .leading, spacing: 1) {
                    Text(model.rundownLabel(item))
                        .fontWeight(isStandby ? .bold : .regular)
                        .lineLimit(1)
                    if let cue = item.pad.flatMap({ model.show.cue(at: $0) }), !item.text.isEmpty {
                        Text(cue.title).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer()
                if isEmpty {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .help("This pad is empty")
                }
            }
            .padding(.vertical, 3)
            .padding(.horizontal, 6)
            .background(isStandby ? Color.green.opacity(0.18) : Color.clear, in: RoundedRectangle(cornerRadius: 6))
            .opacity(isDone ? 0.5 : 1)
            .accessibilityElement(children: .combine)
            .accessibilityValue(isStandby ? "Standby" : isDone ? "Done" : "")
        }
    }

    private var isStandby: Bool { model.standby == index }
    private var isDone: Bool { model.standby.map { index < $0 } ?? true }
    private var isEmpty: Bool { item.pad.map { model.show.cue(at: $0) == nil } ?? false }
    private var icon: String {
        if isStandby { return "arrowtriangle.right.fill" }
        return isDone ? "checkmark" : "circle"
    }
}

/// Edits a note's text or a pad step's label (empty = the pad's title).
private struct RundownTextEditor: View {
    @Environment(\.dismiss) private var dismiss
    let item: RundownItem
    let onSave: (RundownItem) -> Void
    @State private var text = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(item.pad == nil ? (item.id == 0 ? "New Note" : "Edit Note") : "Step Label")
                .font(.headline)
            TextField(item.pad == nil ? "e.g. Timeout 1" : "Leave empty to use the pad's title", text: $text)
                .textFieldStyle(.roundedBorder)
                .frame(width: 300)
                .onSubmit(save)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save", action: save)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .onAppear { text = item.text }
    }

    private func save() {
        var edited = item
        edited.text = text.trimmingCharacters(in: .whitespaces)
        onSave(edited)
        dismiss()
    }
}
