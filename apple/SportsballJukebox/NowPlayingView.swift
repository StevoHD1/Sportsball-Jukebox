import SwiftUI

/// Sidebar listing everything playing, soonest to end first: a big
/// countdown, a bar to click or drag to skip around, what comes next
/// (marker or playlist song), and per-row stop and fade. (The headphone
/// preview has its own player, `PreviewPlayer`.)
struct NowPlayingView: View {
    @EnvironmentObject private var model: AppModel
    var width: Double = 280

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
        .frame(width: width)
        .background(Color.black.opacity(0.12))
    }
}

/// Now Playing as a strip across the top of the window: one card per
/// playing pad, soonest to end first, scrolling sideways.
struct NowPlayingStrip: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Now Playing").font(.headline)
                Text("\(model.nowPlaying.count)")
                    .font(.title2.monospacedDigit().weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .frame(width: 96, alignment: .leading)
            .padding(.leading, 16)
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: 8) {
                    if model.nowPlaying.isEmpty {
                        Text("Nothing playing")
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 8)
                    }
                    ForEach(model.nowPlaying) { item in
                        NowPlayingRow(cue: item.cue, state: item.state, compact: true)
                            .frame(width: 250, height: 96, alignment: .top)
                    }
                }
                .padding(.vertical, 8)
                .padding(.horizontal, 8)
            }
        }
        // Fixed height, so the pads don't jump when something starts.
        .frame(height: 112)
        .background(Color.black.opacity(0.12))
    }
}

private struct NowPlayingRow: View {
    @EnvironmentObject private var model: AppModel
    let cue: CueDoc
    let state: CueStatus
    var compact = false

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
                .font(.system(size: compact ? 22 : 30, weight: .semibold, design: .rounded).monospacedDigit())
                .foregroundStyle(isEnding ? Color.red : Color.primary)
            // Click or drag to skip ahead (or back) in the pad.
            ScrubBar(progress: state.progress, duration: state.duration, tint: isEnding ? .red : .green) {
                model.seek(cue.slot, to: $0)
            }
            if let next = nextText {
                Text(next)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        // In the strip, every card fills the same height.
        .frame(maxWidth: .infinity, maxHeight: compact ? .infinity : nil, alignment: .topLeading)
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

/// Floating mini player shown while previewing (a pad or a library file)
/// in the headphones: scrub to any point, restart, stop.
struct PreviewPlayer: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        // Library files have their own player in the library.
        if let preview = model.status?.preview, !(preview.file != nil && model.showingLibrary) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    Image(systemName: "headphones")
                        .foregroundStyle(.blue)
                    VStack(alignment: .leading, spacing: 0) {
                        Text("Preview")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text(title(preview))
                            .font(.headline)
                            .lineLimit(1)
                    }
                    Spacer()
                    Button {
                        model.stopPreview()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                    .help("Stop and close")
                }
                ScrubBar(
                    progress: preview.duration > 0 ? preview.position / preview.duration : 0,
                    duration: preview.duration,
                    tint: .blue
                ) {
                    model.seekPreview(to: $0)
                }
                HStack {
                    Text(formatTime(preview.position))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        model.seekPreview(to: 0)
                    } label: {
                        Image(systemName: "backward.end.fill")
                    }
                    .help("Play from the start")
                    Button {
                        if preview.playing {
                            model.stopPreview()
                        } else {
                            model.seekPreview(to: preview.position)
                        }
                    } label: {
                        Image(systemName: preview.playing ? "stop.fill" : "play.fill")
                    }
                    .help(preview.playing ? "Stop" : "Play")
                    Spacer()
                    Text("-" + formatTime(max(preview.duration - preview.position, 0)))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.borderless)
            }
            .padding(12)
            .frame(width: 320)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            .shadow(radius: 10)
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Preview player")
        }
    }

    private func title(_ preview: PreviewInfo) -> String {
        if let file = preview.file {
            return URL(fileURLWithPath: file).deletingPathExtension().lastPathComponent
        }
        return model.show.cue(at: preview.slot)?.title ?? "Pad"
    }
}
