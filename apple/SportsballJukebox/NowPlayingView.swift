import SwiftUI

/// Sidebar listing everything playing, soonest to end first: a big
/// countdown, what comes next (marker or playlist song), and per-row
/// stop and fade. Also shows the headphone preview.
struct NowPlayingView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Now Playing").font(.headline)
                Spacer()
                Text("\(model.nowPlaying.count)")
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            .padding(12)
            Divider()
            ScrollView {
                VStack(spacing: 8) {
                    if let preview = model.status?.preview, preview.playing {
                        PreviewRow(preview: preview)
                    }
                    if model.nowPlaying.isEmpty {
                        Text("Nothing playing")
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity)
                            .padding(.top, 24)
                    }
                    ForEach(model.nowPlaying) { item in
                        NowPlayingRow(cue: item.cue, state: item.state)
                    }
                }
                .padding(10)
            }
        }
        .frame(width: 280)
        .background(Color.black.opacity(0.12))
    }
}

private struct NowPlayingRow: View {
    @EnvironmentObject private var model: AppModel
    let cue: CueDoc
    let state: CueStatus

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text(cue.title.isEmpty ? "Pad \(cue.slot + 1)" : cue.title)
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
                Button {
                    model.stop(cue.slot, fade: true)
                } label: {
                    Image(systemName: "speaker.wave.1")
                }
                .buttonStyle(.borderless)
                .help("Fade out")
                Button {
                    model.stop(cue.slot, fade: false)
                } label: {
                    Image(systemName: "stop.fill")
                }
                .buttonStyle(.borderless)
                .help("Stop")
            }
            Text(bigText)
                .font(.system(size: 30, weight: .semibold, design: .rounded).monospacedDigit())
                .foregroundStyle(isEnding ? Color.red : Color.primary)
            ProgressView(value: state.progress)
                .tint(isEnding ? .red : .green)
            if let next = nextText {
                Text(next)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(10)
        .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .combine)
    }

    private var isEnding: Bool {
        (state.remaining ?? .infinity) <= 5
    }

    private var bigText: String {
        if state.holding { return "VAMP" }
        if cue.loop { return "LOOP" }
        guard let remaining = state.remaining else { return formatTime(state.position) }
        return "-" + formatTime(remaining)
    }

    /// The next marker ahead, or a playlist's next song.
    private var nextText: String? {
        if state.holding {
            return "Press the pad (\(cue.hotkey?.uppercased() ?? "or its trigger")) to go"
        }
        if let playlist = cue.playlist {
            guard let next = state.nextItem, playlist.items.indices.contains(next) else { return nil }
            let name = URL(fileURLWithPath: playlist.items[next]).deletingPathExtension().lastPathComponent
            return "Up next: \(name)"
        }
        let ahead = cue.markers
            .filter { $0.time > state.position + 0.01 }
            .min { $0.time < $1.time }
        guard let marker = ahead else { return nil }
        let name = marker.name.isEmpty ? marker.action.kind.label : marker.name
        return "→ \(name) in \(formatTime(marker.time - state.position))"
    }
}

private struct PreviewRow: View {
    @EnvironmentObject private var model: AppModel
    let preview: PreviewInfo

    var body: some View {
        HStack {
            Image(systemName: "headphones")
            VStack(alignment: .leading) {
                Text("Preview: \(model.show.cue(at: preview.slot)?.title ?? "pad")")
                    .lineLimit(1)
                Text(formatTime(preview.position))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button {
                model.stopPreview()
            } label: {
                Image(systemName: "stop.fill")
            }
            .buttonStyle(.borderless)
        }
        .padding(10)
        .background(Color.blue.opacity(0.2), in: RoundedRectangle(cornerRadius: 8))
    }
}
