import AppKit
import SwiftUI
import UniformTypeIdentifiers

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

    /// A preset name, or a custom "#RRGGBB".
    static func color(named name: String?) -> Color {
        guard let name else { return .accentColor }
        if let preset = PadColor(rawValue: name) { return preset.color }
        if let rgb = rgb(hex: name) {
            return Color(.sRGB, red: rgb.0, green: rgb.1, blue: rgb.2, opacity: 1)
        }
        return .accentColor
    }

    static func rgb(hex: String) -> (Double, Double, Double)? {
        guard hex.count == 7, hex.hasPrefix("#"), let value = UInt32(hex.dropFirst(), radix: 16) else {
            return nil
        }
        return (
            Double((value >> 16) & 0xFF) / 255,
            Double((value >> 8) & 0xFF) / 255,
            Double(value & 0xFF) / 255
        )
    }

    /// A small colour swatch for menus (not a template image, so menus
    /// keep its colour).
    static func swatch(_ color: Color) -> NSImage {
        let image = NSImage(size: NSSize(width: 14, height: 14), flipped: false) { rect in
            NSColor(color).setFill()
            NSBezierPath(roundedRect: rect.insetBy(dx: 1, dy: 1), xRadius: 3, yRadius: 3).fill()
            NSColor.black.withAlphaComponent(0.25).setStroke()
            NSBezierPath(roundedRect: rect.insetBy(dx: 1, dy: 1), xRadius: 3, yRadius: 3).stroke()
            return true
        }
        image.isTemplate = false
        return image
    }

    /// "#RRGGBB" for a colour from the colour picker.
    static func hex(_ color: Color) -> String? {
        guard let rgb = NSColor(color).usingColorSpace(.sRGB) else { return nil }
        let r = Int((rgb.redComponent * 255).rounded())
        let g = Int((rgb.greenComponent * 255).rounded())
        let b = Int((rgb.blueComponent * 255).rounded())
        return String(format: "#%02X%02X%02X", min(max(r, 0), 255), min(max(g, 0), 255), min(max(b, 0), 255))
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
    var height: CGFloat = 100
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
                image: cue.flatMap { model.padImage(for: $0) },
                height: height,
                played: model.isPlayed(slot),
                playedColor: model.show.playedColor,
                state: state,
                hasProblems: !problems.isEmpty,
                isBusy: isBusy
            )
            .overlay(shape.strokeBorder(borderColor, lineWidth: isDropTarget || isLearning || state.playing ? 3 : 1))
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .modifier(Hotkey(key: model.suspendHotkeys ? nil : cue?.hotkey))
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
            } else if url.isFileURL, cue != nil, Self.isImage(url) {
                // A picture dropped on a pad becomes its picture.
                model.setImage(url, for: slot)
            } else if url.isFileURL {
                let files = urls.filter(\.isFileURL)
                if NSEvent.modifierFlags.contains(.option) {
                    // ⌥: several files make one playlist pad.
                    model.assignSounds(urls: model.audioFiles(in: files), to: slot)
                } else if files.count > 1 || files.contains(where: { url in
                    var isFolder: ObjCBool = false
                    return FileManager.default.fileExists(atPath: url.path, isDirectory: &isFolder) && isFolder.boolValue
                }) {
                    // A pad per file, filling the bank's empty pads from here.
                    model.addPads(urls: files, bank: slot / max(model.show.slots, 1), startingAt: slot)
                } else {
                    model.assignSound(url: url, to: slot)
                }
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

    static func isImage(_ url: URL) -> Bool {
        UTType(filenameExtension: url.pathExtension)?.conforms(to: .image) ?? false
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
        if cue?.hasActions == true { parts.append("Has actions") }
        if model.isPlayed(slot) { parts.append("Played") }
        if !problems.isEmpty { parts.append("Has problems") }
        return parts.joined(separator: ", ")
    }

    private var borderColor: Color {
        if isDropTarget { return .accentColor }
        if isLearning { return .orange }
        if !problems.isEmpty { return .orange }
        // Playing pads keep their own colour, so a thick white border (and
        // the progress fill) marks them; a green border would be lost on a
        // green pad.
        return state.playing ? .white : Color.secondary.opacity(0.25)
    }

    private var helpText: String {
        guard let cue else { return "Click or drop an audio file to assign a sound" }
        var lines = [cue.title]
        if cue.playMode != .bank { lines.append("⚡ " + cue.playMode.label) }
        lines += cue.onFire.map { "⚡ " + $0.describe(in: model.show) }
        return (lines + problems).joined(separator: "\n")
    }

    @ViewBuilder
    private var menu: some View {
        if let cue {
            Button("Edit Pad…") { model.editing = EditTarget(id: slot) }
            Button("Add to Rundown") { model.addToRundown(pad: slot) }
            Button(model.isPlayed(slot) ? "Unmark as Played" : "Mark as Played") {
                model.setPlayed(slot, !model.isPlayed(slot))
            }
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
            Button("Copy Style") { model.copyStyle(from: slot) }
            Button("Paste Style") { model.pasteStyle(to: slot) }
                .disabled(model.copiedStyle == nil)
            Button("Use Style for New Pads") { model.useStyleAsDefault(from: slot) }
            Button("Choose Picture…") { model.choosePicture(for: slot) }
            if cue.image != nil {
                Button("Remove Picture") { model.setImage(nil, for: slot) }
            }
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
    let image: NSImage?
    let height: CGFloat
    /// Marked as played (shown in the played colour, with a ✓).
    let played: Bool
    let playedColor: String?
    let state: CueStatus
    let hasProblems: Bool
    let isBusy: Bool

    private let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)

    var body: some View {
        ZStack(alignment: .leading) {
            shape.fill(background)

            if let image {
                // The picture fills the pad; a dark fade at the bottom keeps
                // the title readable.
                Color.clear
                    .overlay(
                        Image(nsImage: image)
                            .resizable()
                            .scaledToFill()
                    )
                    .clipShape(shape)
                    .opacity(state.ready || state.playing ? 1 : 0.45)
                LinearGradient(
                    colors: [Color.black.opacity(0.05), Color.black.opacity(0.7)],
                    startPoint: .top,
                    endPoint: .bottom
                )
                .clipShape(shape)
            }

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
                    .lineLimit(compact ? 1 : 2)
                    .opacity(cue == nil ? 0.5 : 1)
                if let song = playlistLine, !compact {
                    Text(song)
                        .font(.caption)
                        .lineLimit(1)
                        .opacity(0.9)
                }
                if let group = cue?.group, !group.isEmpty, !compact {
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
            .padding(compact ? 6 : 10)
        }
        .frame(height: height)
        .animation(.easeOut(duration: 0.12), value: state.playing)
    }

    /// Small pads show just the key, title and time.
    private var compact: Bool { height < 84 }

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
            if played {
                Image(systemName: "checkmark.circle.fill")
                    .font(.caption)
                    .accessibilityLabel("Played")
            }
            if cue?.hasActions == true {
                // Plays over / solo, or runs actions when fired.
                Image(systemName: "bolt.fill")
                    .font(.caption)
                    .padding(3)
                    .background(Color.black.opacity(0.3), in: Circle())
                    .accessibilityLabel("Has actions")
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
        if played, !state.playing, let playedColor {
            return PadColor.color(named: playedColor)
        }
        let base = PadColor.color(named: cue.color)
        // Ready pads show their exact colour (so it can be matched); pads
        // that can't play are faded.
        return state.ready ? base : base.opacity(0.35)
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
