import AppKit
import SwiftUI

/// Pad colours, by the names stored in the show file.
enum PadColor: String, CaseIterable {
    case blue, green, orange, red, purple, pink, teal, gray

    var color: Color {
        switch self {
        case .blue: return .blue
        case .green: return .green
        case .orange: return .orange
        case .red: return .red
        case .purple: return .purple
        case .pink: return .pink
        case .teal: return .teal
        case .gray: return .gray
        }
    }

    static func color(named name: String?) -> Color {
        name.flatMap(PadColor.init(rawValue:))?.color ?? .accentColor
    }
}

/// Dragging a pad onto another moves it; this URL scheme marks pad drags
/// so the same drop target also accepts audio files from Finder.
private let padScheme = "sbjpad"

/// One pad in the grid. Click (or press its key) to fire; drop an audio
/// file on it to assign; drag it onto another pad to swap; right-click for
/// more.
struct CueButton: View {
    @EnvironmentObject private var model: AppModel
    let slot: Int
    @State private var isDropTarget = false

    private let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)

    private var cue: CueDoc? { model.show.cue(at: slot) }
    private var state: CueStatus { model.state(of: slot) }
    private var problems: [String] { model.problems(for: slot) }
    private var isBusy: Bool { model.busySlots.contains(slot) }
    private var isLearning: Bool { model.learningTarget == "cue:\(slot)" }

    var body: some View {
        Button {
            // Option-click previews in the headphones instead.
            if NSEvent.modifierFlags.contains(.option), cue != nil {
                model.preview(slot)
            } else {
                model.trigger(slot)
            }
        } label: {
            PadFace(
                cue: cue,
                state: state,
                hasProblems: !problems.isEmpty,
                isBusy: isBusy
            )
            .overlay(shape.strokeBorder(borderColor, lineWidth: isDropTarget || isLearning ? 3 : 1))
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .modifier(Hotkey(key: cue?.hotkey))
        .contextMenu { menu }
        .draggable(URL(string: "\(padScheme)://\(slot)")!) {
            Text(cue?.title ?? "Pad \(slot + 1)")
                .padding(8)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
        }
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            if url.scheme == padScheme, let from = Int(url.host ?? "") {
                model.move(from: from, to: slot)
            } else if url.isFileURL {
                // Several files make a playlist pad.
                model.assignSounds(urls: urls.filter(\.isFileURL), to: slot)
            } else {
                return false
            }
            return true
        } isTargeted: { isDropTarget = $0 }
        .help(helpText)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(cue.map { "Pad \($0.title)" } ?? "Empty pad")
        .accessibilityValue(accessibilityValue)
        .accessibilityHint(cue == nil ? "Choose a sound" : "Plays the sound from the start")
        .accessibilityAddTraits(.isButton)
    }

    private var accessibilityValue: String {
        guard cue != nil else { return "" }
        var parts: [String] = []
        if state.holding {
            parts.append("Vamping, press to continue")
        } else if state.playing {
            parts.append("Playing, \(formatTime(max(state.duration - state.position, 0))) remaining")
        } else if !state.ready {
            parts.append("Not ready")
        }
        if let key = cue?.hotkey { parts.append("Key \(key.uppercased())") }
        if !problems.isEmpty { parts.append("Has problems") }
        return parts.joined(separator: ", ")
    }

    private var borderColor: Color {
        if isDropTarget { return .accentColor }
        if isLearning { return .orange }
        if !problems.isEmpty { return .orange }
        return state.playing ? .green : Color.secondary.opacity(0.25)
    }

    private var helpText: String {
        guard let cue else { return "Click or drop an audio file to assign a sound" }
        return ([cue.title] + problems).joined(separator: "\n")
    }

    @ViewBuilder
    private var menu: some View {
        if let cue {
            Button("Edit Pad…") { model.editing = EditTarget(id: slot) }
            Button("Preview in Headphones (⌥-click)") { model.preview(slot) }
                .disabled(!model.canPreview)
            if cue.playlist != nil {
                Button("Next Song") { model.skip(slot) }
            }
            Button(cue.playlist == nil ? "Replace Sound…" : "Replace Songs…") { model.chooseSound(for: slot) }
            Toggle("Loop", isOn: Binding(
                get: { cue.loop },
                set: { model.setLoop(slot, $0) }
            ))
            Divider()
            Button(cue.midi == nil ? "Learn MIDI Trigger" : "Re-learn MIDI Trigger") {
                model.learnMIDI("cue:\(slot)")
            }
            if let midi = cue.midi {
                Button("Clear MIDI (\(midi.label))") { model.clearMIDI("cue:\(slot)") }
            }
            Divider()
            Button("Stop") { model.stop(slot, fade: false) }
            Button("Fade Out") { model.stop(slot, fade: true) }
            Divider()
            Button("Clear Pad", role: .destructive) { model.clear(slot) }
        } else {
            Button("Load Sound…") { model.chooseSound(for: slot) }
        }
    }
}

private struct PadFace: View {
    let cue: CueDoc?
    let state: CueStatus
    let hasProblems: Bool
    let isBusy: Bool

    private let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)

    var body: some View {
        ZStack(alignment: .leading) {
            shape.fill(background)

            GeometryReader { geometry in
                Rectangle()
                    .fill(isEnding ? Color.red.opacity(0.55) : Color.white.opacity(0.22))
                    .frame(width: geometry.size.width * state.progress)
            }
            .clipShape(shape)

            VStack(alignment: .leading, spacing: 4) {
                topRow
                Spacer(minLength: 0)
                Text(cue?.title ?? "Empty")
                    .font(.headline)
                    .lineLimit(2)
                    .opacity(cue == nil ? 0.5 : 1)
                if let song = playlistLine {
                    Text(song)
                        .font(.caption)
                        .lineLimit(1)
                        .opacity(0.9)
                }
                if let group = cue?.group, !group.isEmpty {
                    Text(group.uppercased())
                        .font(.caption2.weight(.semibold))
                        .opacity(0.75)
                }
                if cue != nil {
                    Text(timeText)
                        .font(.caption.monospacedDigit())
                        .opacity(0.85)
                }
            }
            .foregroundStyle(cue == nil ? Color.secondary : Color.white)
            .padding(10)
        }
        .frame(height: 100)
        .animation(.easeOut(duration: 0.12), value: state.playing)
    }

    private var topRow: some View {
        HStack(spacing: 6) {
            if let key = cue?.hotkey, !key.isEmpty {
                Text(key.uppercased())
                    .font(.caption.monospaced().weight(.semibold))
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .background(Color.black.opacity(0.25), in: RoundedRectangle(cornerRadius: 4))
            }
            Spacer(minLength: 0)
            if state.holding {
                Text("VAMP")
                    .font(.caption2.weight(.bold))
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .background(Color.orange, in: Capsule())
                    .foregroundStyle(.black)
            }
            if hasProblems {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.yellow)
            }
            if cue?.duck != nil {
                Image(systemName: "speaker.wave.1")
                    .help("Ducks other pads while playing")
            }
            if let cue, !cue.markers.isEmpty {
                Image(systemName: "flag.fill")
            }
            if cue?.midi != nil {
                Image(systemName: "pianokeys")
            }
            if cue?.loop == true {
                Image(systemName: "repeat")
            }
            if isBusy {
                ProgressView().controlSize(.small)
            }
        }
    }

    /// In its last 5 seconds (and not about to loop or carry on).
    private var isEnding: Bool {
        guard let remaining = state.remaining else { return false }
        if cue?.playlist?.autoAdvance == true { return false }
        return remaining <= 5
    }

    /// "♫ 3/12 · Song name" for playlist pads.
    private var playlistLine: String? {
        guard let items = cue?.playlist?.items, !items.isEmpty else { return nil }
        guard let index = state.item, items.indices.contains(index) else {
            return "♫ \(items.count) songs"
        }
        let name = URL(fileURLWithPath: items[index]).deletingPathExtension().lastPathComponent
        return "♫ \(index + 1)/\(items.count) · \(name)"
    }

    private var background: Color {
        guard let cue else { return Color.secondary.opacity(0.12) }
        if state.playing { return Color.green.opacity(0.85) }
        let base = PadColor.color(named: cue.color)
        return state.ready ? base.opacity(0.75) : base.opacity(0.35)
    }

    private var timeText: String {
        if state.playing {
            return "\(formatTime(state.position)) / -\(formatTime(max(state.duration - state.position, 0)))"
        }
        return state.ready ? formatTime(state.duration) : "Not ready"
    }
}

func formatTime(_ seconds: Double) -> String {
    let clamped = max(seconds, 0)
    let total = Int(clamped.rounded(.down))
    let tenths = Int((clamped - Double(total)) * 10)
    return String(format: "%d:%02d.%d", total / 60, total % 60, tenths)
}

/// Binds a bare key (no modifiers) to the pad.
private struct Hotkey: ViewModifier {
    let key: String?

    @ViewBuilder
    func body(content: Content) -> some View {
        if let character = key?.lowercased().first {
            content.keyboardShortcut(KeyEquivalent(character), modifiers: [])
        } else {
            content
        }
    }
}
