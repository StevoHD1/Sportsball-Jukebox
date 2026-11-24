import SwiftUI

/// One pad in the cue grid. Click (or press its key) to fire; drop an audio
/// file on it to assign; right-click for more.
struct CueButton: View {
    @EnvironmentObject private var model: AppModel
    let cue: Cue
    @State private var isDropTarget = false

    private let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)

    var body: some View {
        Button {
            model.trigger(cue.id)
        } label: {
            pad
        }
        .buttonStyle(.plain)
        .modifier(Hotkey(key: cue.hotkey))
        .contextMenu { menu }
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            model.loadSound(url: url, into: cue.id)
            return true
        } isTargeted: { isDropTarget = $0 }
        .help(cue.title ?? "Click or drop an audio file to assign a sound")
    }

    private var pad: some View {
        ZStack(alignment: .leading) {
            shape.fill(background)

            // Playback progress.
            GeometryReader { geometry in
                Rectangle()
                    .fill(Color.white.opacity(0.22))
                    .frame(width: geometry.size.width * cue.progress)
            }
            .clipShape(shape)

            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    if let key = cue.hotkey {
                        Text(String(key).uppercased())
                            .font(.caption.monospaced().weight(.semibold))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Color.black.opacity(0.25), in: RoundedRectangle(cornerRadius: 4))
                    }
                    Spacer(minLength: 0)
                    if cue.looping {
                        Image(systemName: "repeat")
                    }
                    if cue.isLoading {
                        ProgressView().controlSize(.small)
                    }
                }
                Spacer(minLength: 0)
                Text(cue.title ?? "Empty")
                    .font(.headline)
                    .lineLimit(2)
                    .opacity(cue.isAssigned ? 1 : 0.5)
                if cue.isAssigned {
                    Text(timeText)
                        .font(.caption.monospacedDigit())
                        .opacity(0.85)
                }
            }
            .foregroundStyle(cue.isAssigned ? Color.white : Color.secondary)
            .padding(10)
        }
        .frame(height: 100)
        .overlay(shape.strokeBorder(borderColor, lineWidth: isDropTarget ? 3 : 1))
        .contentShape(shape)
        .animation(.easeOut(duration: 0.12), value: cue.playing)
    }

    private var background: Color {
        if !cue.isAssigned { return Color.secondary.opacity(0.12) }
        return cue.playing ? Color.green.opacity(0.85) : Color.accentColor.opacity(0.75)
    }

    private var borderColor: Color {
        if isDropTarget { return .accentColor }
        return cue.playing ? .green : Color.secondary.opacity(0.25)
    }

    private var timeText: String {
        if cue.playing {
            return "\(format(cue.position)) / -\(format(max(cue.duration - cue.position, 0)))"
        }
        return format(cue.duration)
    }

    private func format(_ seconds: Double) -> String {
        let total = Int(seconds.rounded(.down))
        let tenths = Int((seconds - Double(total)) * 10)
        return String(format: "%d:%02d.%d", total / 60, total % 60, tenths)
    }

    @ViewBuilder
    private var menu: some View {
        Button("Load Sound…") { model.chooseSound(for: cue.id) }
        if cue.isAssigned {
            Toggle("Loop", isOn: Binding(
                get: { cue.looping },
                set: { model.setLoop(cue.id, $0) }
            ))
            Divider()
            Button("Stop") { model.stop(cue.id, fade: false) }
            Button("Fade Out") { model.stop(cue.id, fade: true) }
            Divider()
            Button("Clear", role: .destructive) { model.clear(cue.id) }
        }
    }
}

/// Binds a bare key (no modifiers) to the cue.
private struct Hotkey: ViewModifier {
    let key: Character?

    @ViewBuilder
    func body(content: Content) -> some View {
        if let key {
            content.keyboardShortcut(KeyEquivalent(key), modifiers: [])
        } else {
            content
        }
    }
}
