import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// What is selected in the editor.
enum EditorSelection: Equatable {
    case clip(Int)
    case marker(UInt64)
}

/// Edits one pad: its settings, its clips on a timeline, and markers with
/// actions (jump, loop, pause, stop, fade). Changes are applied when you
/// play from the editor or press Done.
struct CueEditorView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let slot: Int

    @State private var draft: CueDoc
    @State private var original: CueDoc
    @State private var selection: EditorSelection?
    @State private var cursor: Double = 0
    @State private var pointsPerSecond: Double = 60
    @State private var loaded = false

    init(slot: Int) {
        self.slot = slot
        _draft = State(initialValue: CueDoc(slot: slot))
        _original = State(initialValue: CueDoc(slot: slot))
    }

    private var isDirty: Bool { draft != original }
    private var state: CueStatus { model.state(of: slot) }

    var body: some View {
        VStack(spacing: 0) {
            EditorHeader(draft: $draft, outputs: model.show.outputs, conflict: hotkeyConflict)
                .padding([.horizontal, .top], 16)
            PlaybackRow(draft: $draft, channels: outputChannels)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
            Divider()
            if draft.playlist != nil {
                PlaylistEditor(
                    draft: $draft,
                    currentItem: state.playing ? state.item : nil,
                    onPlay: { play(from: 0) },
                    onSkip: {
                        apply()
                        model.skip(slot)
                    },
                    onStop: { model.stop(slot, fade: false) }
                )
                .frame(minHeight: 390)
            } else {
                EditorToolbar(
                    cursor: cursor,
                    pointsPerSecond: $pointsPerSecond,
                    isPlaying: state.playing,
                    onAddClip: addClip,
                    onAddMarker: addMarker,
                    onPlayFromCursor: { play(from: cursor) },
                    onPlayFromStart: { play(from: 0) },
                    onStop: { model.stop(slot, fade: false) }
                )
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                CueTimeline(
                    draft: $draft,
                    selection: $selection,
                    cursor: $cursor,
                    pointsPerSecond: pointsPerSecond,
                    playhead: state.playing ? state.position : nil
                )
                .frame(minHeight: 220)
                Divider()
                InspectorView(draft: $draft, selection: $selection)
                    .frame(height: 170)
                    .padding(.horizontal, 16)
            }
            Divider()
            footer
                .padding(16)
        }
        .frame(minWidth: 980, minHeight: 680)
        .onAppear(perform: load)
        .onChange(of: draft.clips.map(\.file)) { _, files in
            files.forEach(model.loadAudioInfo(for:))
        }
    }

    private var footer: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(model.problems(for: slot), id: \.self) { problem in
                    Label(problem, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.callout)
                }
                if isDirty {
                    Text("Unapplied changes — they apply when you play or press Done.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
            Button("Done") {
                apply()
                dismiss()
            }
            .keyboardShortcut(.defaultAction)
        }
    }

    /// Channel count of the pad's output device (8 if not known yet).
    private var outputChannels: Int {
        let outputs = model.status?.outputs ?? []
        guard outputs.indices.contains(draft.output) else { return 8 }
        let channels = outputs[draft.output].channels
        return channels > 0 ? channels : 8
    }

    private var hotkeyConflict: String? {
        guard let key = draft.hotkey?.lowercased(), !key.isEmpty else { return nil }
        return model.show.cues
            .first { $0.slot != slot && $0.hotkey?.lowercased() == key }
            .map { "Also used by \"\($0.title)\"" }
    }

    private func load() {
        guard !loaded else { return }
        loaded = true
        if let cue = model.show.cue(at: slot) {
            draft = cue
            original = cue
        }
        draft.clips.map(\.file).forEach(model.loadAudioInfo(for:))
    }

    private func apply() {
        guard isDirty else { return }
        model.updateCue(draft)
        original = draft
    }

    private func play(from seconds: Double) {
        apply()
        // A just-applied cue loads in the background; give it a moment.
        let delay: UInt64 = model.busySlots.contains(slot) ? 300_000_000 : 0
        Task { @MainActor in
            if delay > 0 { try? await Task.sleep(nanoseconds: delay) }
            model.play(slot, from: seconds)
        }
    }

    private func addClip() {
        let panel = NSOpenPanel()
        panel.title = "Add a clip"
        panel.allowedContentTypes = [.audio]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        var clip = ClipDoc(file: url.path)
        clip.start = TimelineMath.contentEnd(of: draft, durations: model.durations)
        draft.clips.append(clip)
        selection = .clip(draft.clips.count - 1)
        model.loadAudioInfo(for: url.path)
    }

    private func addMarker() {
        let marker = MarkerDoc(
            id: draft.nextMarkerID,
            name: "Marker \(draft.markers.count + 1)",
            time: cursor,
            action: ActionDoc(kind: .loop, to: TargetDoc(time: 0, marker: nil), count: nil, seconds: nil)
        )
        draft.markers.append(marker)
        selection = .marker(marker.id)
    }
}

// MARK: - Header

private struct EditorHeader: View {
    @Binding var draft: CueDoc
    let outputs: [OutputDoc]
    let conflict: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 16) {
                TextField("Title", text: $draft.title)
                    .textFieldStyle(.roundedBorder)
                    .font(.title3)
                    .frame(minWidth: 220, maxWidth: .infinity)

                HStack(spacing: 6) {
                    Text("Key")
                    TextField("", text: hotkey)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 36)
                }

                Picker("Output", selection: $draft.output) {
                    ForEach(Array(outputs.enumerated()), id: \.offset) { index, output in
                        Text(output.name).tag(index)
                    }
                }
                .frame(width: 200)

                Picker("Colour", selection: color) {
                    ForEach(PadColor.allCases, id: \.self) { c in
                        Text(c.rawValue.capitalized).tag(c.rawValue)
                    }
                }
                .frame(width: 150)
            }

            HStack(spacing: 20) {
                HStack(spacing: 6) {
                    Text("Volume")
                    Slider(value: gain, in: 0...2)
                        .frame(width: 140)
                    Text(gainLabel)
                        .font(.caption.monospacedDigit())
                        .frame(width: 54, alignment: .leading)
                }

                Toggle("Loop", isOn: $draft.loop)

                MidiField(trigger: $draft.midi, slot: draft.slot)

                Spacer()

                if let conflict {
                    Label(conflict, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
            }
        }
    }

    /// A single lowercase character, or none.
    private var hotkey: Binding<String> {
        Binding(
            get: { draft.hotkey ?? "" },
            set: { text in
                let key = text.trimmingCharacters(in: .whitespaces).lowercased().suffix(1)
                draft.hotkey = key.isEmpty ? nil : String(key)
            }
        )
    }

    private var gain: Binding<Double> {
        Binding(get: { Double(draft.gain) }, set: { draft.gain = Float($0) })
    }

    private var gainLabel: String {
        draft.gain <= 0.0001 ? "-∞ dB" : String(format: "%+.1f dB", 20 * log10(Double(draft.gain)))
    }

    private var color: Binding<String> {
        Binding(
            get: { draft.color ?? PadColor.blue.rawValue },
            set: { draft.color = $0 }
        )
    }
}

// MARK: - Toolbar

private struct EditorToolbar: View {
    let cursor: Double
    @Binding var pointsPerSecond: Double
    let isPlaying: Bool
    let onAddClip: () -> Void
    let onAddMarker: () -> Void
    let onPlayFromCursor: () -> Void
    let onPlayFromStart: () -> Void
    let onStop: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button(action: onAddClip) { Label("Add Clip…", systemImage: "waveform.badge.plus") }
            Button(action: onAddMarker) { Label("Add Marker", systemImage: "flag") }
                .help("Adds a marker at the cursor")
            Divider().frame(height: 20)
            Button(action: onPlayFromStart) { Image(systemName: "backward.end.fill") }
                .help("Play from the start")
            Button(action: onPlayFromCursor) { Image(systemName: "play.fill") }
                .help("Play from the cursor")
            Button(action: onStop) { Image(systemName: "stop.fill") }
                .disabled(!isPlaying)
            Text("Cursor \(formatTime(cursor))")
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
            Spacer()
            Image(systemName: "minus.magnifyingglass")
            Slider(value: $pointsPerSecond, in: 10...400)
                .frame(width: 140)
            Image(systemName: "plus.magnifyingglass")
        }
    }
}

// MARK: - Timeline

enum TimelineMath {
    static func length(of clip: ClipDoc, durations: [String: Double]) -> Double {
        if let length = clip.length { return length }
        guard let duration = durations[clip.file] else { return 0 }
        return max(duration - clip.offset, 0)
    }

    static func contentEnd(of cue: CueDoc, durations: [String: Double]) -> Double {
        let clipEnds = cue.clips.map { $0.start + length(of: $0, durations: durations) }
        let markerTimes = cue.markers.map(\.time)
        return (clipEnds + markerTimes).max() ?? 0
    }

    /// Where a jump/loop marker goes, in seconds.
    static func target(of marker: MarkerDoc, in cue: CueDoc) -> Double? {
        guard marker.action.kind.needsTarget, let to = marker.action.to else { return nil }
        if let id = to.marker {
            return cue.markers.first { $0.id == id }?.time
        }
        return to.time
    }

    /// Snaps to 1/100 s.
    static func snap(_ seconds: Double) -> Double {
        (max(seconds, 0) * 100).rounded() / 100
    }
}

private struct CueTimeline: View {
    @EnvironmentObject private var model: AppModel
    @Binding var draft: CueDoc
    @Binding var selection: EditorSelection?
    @Binding var cursor: Double
    let pointsPerSecond: Double
    let playhead: Double?

    private var width: CGFloat {
        let end = TimelineMath.contentEnd(of: draft, durations: model.durations)
        return max(CGFloat((end + 4) * pointsPerSecond), 800)
    }

    var body: some View {
        GeometryReader { viewport in
        ScrollView([.horizontal, .vertical]) {
            VStack(alignment: .leading, spacing: 0) {
                RulerView(width: width, pointsPerSecond: pointsPerSecond) { seconds in
                    cursor = TimelineMath.snap(seconds)
                }
                MarkerLane(
                    draft: $draft,
                    selection: $selection,
                    width: width,
                    pointsPerSecond: pointsPerSecond
                )
                ForEach(Array(draft.clips.enumerated()), id: \.offset) { index, clip in
                    ClipLane(
                        clip: clip,
                        length: TimelineMath.length(of: clip, durations: model.durations),
                        fileDuration: model.durations[clip.file],
                        waveform: model.waveforms[clip.file],
                        width: width,
                        pointsPerSecond: pointsPerSecond,
                        isSelected: selection == .clip(index),
                        onSelect: { selection = .clip(index) },
                        onMove: { start in
                            if draft.clips.indices.contains(index) {
                                draft.clips[index].start = TimelineMath.snap(start)
                            }
                        }
                    )
                }
                if draft.clips.isEmpty {
                    Text("No clips. Use Add Clip… to place audio on the timeline.")
                        .foregroundStyle(.secondary)
                        .padding(20)
                }
            }
            .overlay(alignment: .topLeading) {
                ZStack(alignment: .topLeading) {
                    ForEach(draft.markers) { marker in
                        VLine(color: .yellow.opacity(0.5))
                            .offset(x: CGFloat(marker.time * pointsPerSecond))
                    }
                    VLine(color: .accentColor)
                        .offset(x: CGFloat(cursor * pointsPerSecond))
                    if let playhead {
                        VLine(color: .green, width: 2)
                            .offset(x: CGFloat(playhead * pointsPerSecond))
                    }
                }
                .allowsHitTesting(false)
            }
            // Fill the visible height so short content sits at the top
            // instead of being centred.
            .frame(minHeight: viewport.size.height, alignment: .topLeading)
        }
        }
        .background(Color.black.opacity(0.15))
    }
}

private struct VLine: View {
    let color: Color
    var width: CGFloat = 1

    var body: some View {
        Rectangle()
            .fill(color)
            .frame(width: width)
            .frame(maxHeight: .infinity)
    }
}

private struct RulerView: View {
    let width: CGFloat
    let pointsPerSecond: Double
    let onClick: (Double) -> Void

    var body: some View {
        Canvas { context, size in
            let step = tickStep
            var t = 0.0
            while CGFloat(t * pointsPerSecond) < size.width {
                let x = CGFloat(t * pointsPerSecond)
                var tick = Path()
                tick.move(to: CGPoint(x: x, y: size.height - 8))
                tick.addLine(to: CGPoint(x: x, y: size.height))
                context.stroke(tick, with: .color(.secondary), lineWidth: 1)
                context.draw(
                    Text(formatTime(t)).font(.caption2).foregroundColor(.secondary),
                    at: CGPoint(x: x + 3, y: 2),
                    anchor: .topLeading
                )
                t += step
            }
        }
        .frame(width: width, height: 26)
        .background(Color.secondary.opacity(0.1))
        .contentShape(Rectangle())
        .gesture(SpatialTapGesture().onEnded { value in
            onClick(Double(value.location.x) / pointsPerSecond)
        })
        .help("Click to place the cursor")
    }

    /// Seconds between labelled ticks, keeping labels ~80 pt apart.
    private var tickStep: Double {
        let candidates: [Double] = [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120]
        return candidates.first { $0 * pointsPerSecond >= 80 } ?? 300
    }
}

private struct MarkerLane: View {
    @Binding var draft: CueDoc
    @Binding var selection: EditorSelection?
    let width: CGFloat
    let pointsPerSecond: Double

    var body: some View {
        ZStack(alignment: .topLeading) {
            // Arrows from jump/loop markers to their targets.
            Canvas { context, size in
                for marker in draft.markers {
                    guard let target = TimelineMath.target(of: marker, in: draft) else { continue }
                    let from = CGPoint(x: marker.time * pointsPerSecond, y: size.height - 4)
                    let to = CGPoint(x: target * pointsPerSecond, y: size.height - 4)
                    var arc = Path()
                    arc.move(to: from)
                    arc.addQuadCurve(
                        to: to,
                        control: CGPoint(x: (from.x + to.x) / 2, y: size.height - 26)
                    )
                    context.stroke(arc, with: .color(.yellow.opacity(0.7)), lineWidth: 1.5)
                    var head = Path()
                    head.addEllipse(in: CGRect(x: to.x - 3, y: to.y - 3, width: 6, height: 6))
                    context.fill(head, with: .color(.yellow))
                }
            }
            .frame(width: width, height: 44)

            ForEach(draft.markers) { marker in
                MarkerFlag(
                    marker: marker,
                    pointsPerSecond: pointsPerSecond,
                    isSelected: selection == .marker(marker.id),
                    onSelect: { selection = .marker(marker.id) },
                    onMove: { time in
                        if let index = draft.markers.firstIndex(where: { $0.id == marker.id }) {
                            draft.markers[index].time = TimelineMath.snap(time)
                        }
                    }
                )
            }
        }
        .frame(width: width, height: 44, alignment: .topLeading)
    }
}

private struct MarkerFlag: View {
    let marker: MarkerDoc
    let pointsPerSecond: Double
    let isSelected: Bool
    let onSelect: () -> Void
    let onMove: (Double) -> Void
    @State private var drag: CGFloat = 0

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: icon)
            Text(marker.name.isEmpty ? marker.action.kind.label : marker.name)
                .lineLimit(1)
        }
        .font(.caption.weight(.semibold))
        .padding(.horizontal, 5)
        .padding(.vertical, 2)
        .background(
            (isSelected ? Color.yellow : Color.yellow.opacity(0.6)),
            in: RoundedRectangle(cornerRadius: 4)
        )
        .foregroundStyle(.black)
        .offset(x: CGFloat(marker.time * pointsPerSecond) + drag, y: 2)
        .gesture(
            DragGesture(minimumDistance: 2)
                .onChanged { drag = $0.translation.width }
                .onEnded { value in
                    onMove(marker.time + Double(value.translation.width) / pointsPerSecond)
                    drag = 0
                }
        )
        .simultaneousGesture(TapGesture().onEnded(onSelect))
        .help("Drag to move; click to edit")
    }

    private var icon: String {
        switch marker.action.kind {
        case .jump: return "arrow.uturn.right"
        case .loop: return "repeat"
        case .pause: return "pause.fill"
        case .stop: return "stop.fill"
        case .fadeOut: return "speaker.wave.1"
        }
    }
}

private struct ClipLane: View {
    let clip: ClipDoc
    let length: Double
    let fileDuration: Double?
    let waveform: [Float]?
    let width: CGFloat
    let pointsPerSecond: Double
    let isSelected: Bool
    let onSelect: () -> Void
    let onMove: (Double) -> Void
    @State private var drag: CGFloat = 0

    var body: some View {
        ZStack(alignment: .topLeading) {
            Color.clear.frame(width: width, height: 72)
            ClipBox(
                clip: clip,
                length: length,
                fileDuration: fileDuration,
                waveform: waveform,
                isSelected: isSelected
            )
            .frame(width: max(CGFloat(length * pointsPerSecond), 6), height: 62)
            .offset(x: CGFloat(clip.start * pointsPerSecond) + drag, y: 5)
            .gesture(
                DragGesture(minimumDistance: 2)
                    .onChanged { drag = $0.translation.width }
                    .onEnded { value in
                        onMove(clip.start + Double(value.translation.width) / pointsPerSecond)
                        drag = 0
                    }
            )
            .simultaneousGesture(TapGesture().onEnded(onSelect))
        }
        .frame(width: width, height: 72, alignment: .topLeading)
    }
}

private struct ClipBox: View {
    let clip: ClipDoc
    let length: Double
    let fileDuration: Double?
    let waveform: [Float]?
    let isSelected: Bool

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 6)
                .fill(Color.accentColor.opacity(isSelected ? 0.55 : 0.35))
            WaveformShape(clip: clip, length: length, fileDuration: fileDuration, peaks: waveform)
            FadeShape(clip: clip, length: length)
            Text(clip.fileName)
                .font(.caption.weight(.semibold))
                .lineLimit(1)
                .padding(4)
        }
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(isSelected ? Color.white : Color.clear, lineWidth: 2)
        )
        .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

/// Draws the part of the file's waveform the clip plays.
private struct WaveformShape: View {
    let clip: ClipDoc
    let length: Double
    let fileDuration: Double?
    let peaks: [Float]?

    var body: some View {
        Canvas { context, size in
            guard let peaks, let duration = fileDuration, duration > 0, length > 0 else { return }
            let buckets = peaks.count / 2
            guard buckets > 0, size.width > 0 else { return }
            let mid = size.height / 2
            let scale = size.height / 2 * 0.9 * CGFloat(clip.gain)
            var path = Path()
            var x: CGFloat = 0
            while x < size.width {
                let t = clip.offset + Double(x / size.width) * length
                let bucket = min(max(Int(t / duration * Double(buckets)), 0), buckets - 1)
                let lo = CGFloat(peaks[bucket * 2])
                let hi = CGFloat(peaks[bucket * 2 + 1])
                path.move(to: CGPoint(x: x, y: mid - hi * scale))
                path.addLine(to: CGPoint(x: x, y: mid - lo * scale + 0.5))
                x += 1
            }
            context.stroke(path, with: .color(.white.opacity(0.8)), lineWidth: 1)
        }
    }
}

/// Shades the fade-in and fade-out regions.
private struct FadeShape: View {
    let clip: ClipDoc
    let length: Double

    var body: some View {
        Canvas { context, size in
            guard length > 0 else { return }
            let pps = size.width / CGFloat(length)
            if clip.fadeIn > 0 {
                var p = Path()
                p.move(to: .zero)
                p.addLine(to: CGPoint(x: CGFloat(clip.fadeIn) * pps, y: 0))
                p.addLine(to: CGPoint(x: 0, y: size.height))
                p.closeSubpath()
                context.fill(p, with: .color(.black.opacity(0.35)))
            }
            if clip.fadeOut > 0 {
                var p = Path()
                p.move(to: CGPoint(x: size.width, y: 0))
                p.addLine(to: CGPoint(x: size.width - CGFloat(clip.fadeOut) * pps, y: 0))
                p.addLine(to: CGPoint(x: size.width, y: size.height))
                p.closeSubpath()
                context.fill(p, with: .color(.black.opacity(0.35)))
            }
        }
        .allowsHitTesting(false)
    }
}

// MARK: - Inspector

private struct InspectorView: View {
    @EnvironmentObject private var model: AppModel
    @Binding var draft: CueDoc
    @Binding var selection: EditorSelection?

    var body: some View {
        Group {
            switch selection {
            case .some(.clip(let index)) where draft.clips.indices.contains(index):
                ClipInspector(draft: $draft, index: index, onRemove: {
                    draft.clips.remove(at: index)
                    selection = nil
                })
            case .some(.marker(let id)) where draft.markers.contains(where: { $0.id == id }):
                MarkerInspector(draft: $draft, id: id, onRemove: {
                    draft.markers.removeAll { $0.id == id }
                    selection = nil
                })
            default:
                Text("Select a clip or marker to edit it. Click the ruler to move the cursor; drag clips and markers to move them.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding(.vertical, 10)
    }
}

/// A number field in seconds.
private struct SecondsField: View {
    let label: String
    @Binding var value: Double

    var body: some View {
        HStack(spacing: 4) {
            Text(label)
            TextField("", value: $value, format: .number.precision(.fractionLength(0...3)))
                .textFieldStyle(.roundedBorder)
                .frame(width: 80)
            Text("s").foregroundStyle(.secondary)
        }
    }
}

private struct ClipInspector: View {
    @EnvironmentObject private var model: AppModel
    @Binding var draft: CueDoc
    let index: Int
    let onRemove: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(draft.clips[index].fileName, systemImage: "waveform")
                    .font(.headline)
                if let duration = model.durations[draft.clips[index].file] {
                    Text("(\(formatTime(duration)) file)")
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Remove Clip", role: .destructive, action: onRemove)
            }
            HStack(spacing: 18) {
                SecondsField(label: "Start", value: field(\.start, 0))
                SecondsField(label: "Skip into file", value: field(\.offset, 0))
                Toggle("Play to end", isOn: toEnd)
                if !toEnd.wrappedValue {
                    SecondsField(label: "Length", value: length)
                }
            }
            HStack(spacing: 18) {
                SecondsField(label: "Fade in", value: field(\.fadeIn, 0))
                SecondsField(label: "Fade out", value: field(\.fadeOut, 0))
                Picker("Curve", selection: field(\.fadeCurve, .linear)) {
                    ForEach(CurveDoc.allCases, id: \.self) { curve in
                        Text(curve.label).tag(curve)
                    }
                }
                .frame(width: 180)
                HStack(spacing: 6) {
                    Text("Clip volume")
                    Slider(value: clipGain, in: 0...2).frame(width: 100)
                }
            }
        }
    }

    private func field<T>(_ keyPath: WritableKeyPath<ClipDoc, T>, _ fallback: T) -> Binding<T> {
        Binding(
            get: { draft.clips.indices.contains(index) ? draft.clips[index][keyPath: keyPath] : fallback },
            set: { if draft.clips.indices.contains(index) { draft.clips[index][keyPath: keyPath] = $0 } }
        )
    }

    private var toEnd: Binding<Bool> {
        Binding(
            get: { draft.clips.indices.contains(index) && draft.clips[index].length == nil },
            set: { playToEnd in
                guard draft.clips.indices.contains(index) else { return }
                if playToEnd {
                    draft.clips[index].length = nil
                } else {
                    let clip = draft.clips[index]
                    draft.clips[index].length = TimelineMath.length(of: clip, durations: model.durations)
                }
            }
        )
    }

    private var length: Binding<Double> {
        Binding(
            get: { draft.clips.indices.contains(index) ? (draft.clips[index].length ?? 0) : 0 },
            set: { if draft.clips.indices.contains(index) { draft.clips[index].length = max($0, 0) } }
        )
    }

    private var clipGain: Binding<Double> {
        Binding(
            get: { draft.clips.indices.contains(index) ? Double(draft.clips[index].gain) : 1 },
            set: { if draft.clips.indices.contains(index) { draft.clips[index].gain = Float($0) } }
        )
    }
}

private struct MarkerInspector: View {
    @Binding var draft: CueDoc
    let id: UInt64
    let onRemove: () -> Void

    private var index: Int? { draft.markers.firstIndex { $0.id == id } }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 18) {
                Image(systemName: "flag.fill").foregroundStyle(.yellow)
                TextField("Name", text: field(\.name, ""))
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 180)
                SecondsField(label: "At", value: field(\.time, 0))
                Picker("Action", selection: kind) {
                    ForEach(ActionKind.allCases, id: \.self) { kind in
                        Text(kind.label).tag(kind)
                    }
                }
                .frame(width: 180)
                Spacer()
                Button("Remove Marker", role: .destructive, action: onRemove)
            }
            actionDetails
            Text(explanation)
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var actionDetails: some View {
        switch kind.wrappedValue {
        case .jump, .loop:
            HStack(spacing: 18) {
                Picker("Go to", selection: targetChoice) {
                    Text("A time").tag(UInt64(0))
                    ForEach(draft.markers.filter { $0.id != id }) { other in
                        Text("Marker \"\(other.name)\"").tag(other.id)
                    }
                }
                .frame(width: 240)
                if targetChoice.wrappedValue == 0 {
                    SecondsField(label: "Time", value: targetTime)
                }
                if kind.wrappedValue == .loop {
                    Picker("Repeat", selection: loopMode) {
                        Text("A number of times").tag(LoopMode.times)
                        Text("Until the pad is pressed (vamp)").tag(LoopMode.vamp)
                        Text("Forever").tag(LoopMode.forever)
                    }
                    .frame(width: 300)
                    if loopMode.wrappedValue == .times {
                        Stepper("\(count.wrappedValue)×", value: count, in: 1...999)
                    }
                }
            }
        case .fadeOut:
            SecondsField(label: "Fade length", value: fadeSeconds)
        case .pause, .stop:
            EmptyView()
        }
    }

    private var explanation: String {
        switch kind.wrappedValue {
        case .jump: return "When the playhead reaches this marker, it jumps to the target and keeps playing."
        case .loop:
            switch loopMode.wrappedValue {
            case .times: return "Jumps back to the target, then continues past this marker once the repeats are used up."
            case .vamp: return "Vamp: loops until the pad is pressed again (or its key, MIDI or OSC), then plays on past this marker. Put the marker at the end of a bar for a musical exit."
            case .forever: return "Loops until you stop the pad."
            }
        case .pause: return "Playback pauses here. Play from the editor or fire the pad to restart."
        case .stop: return "Playback stops here and returns to the start."
        case .fadeOut: return "Everything on this pad fades to silence, then stops."
        }
    }

    // Bindings that tolerate the marker disappearing.

    private func field<T>(_ keyPath: WritableKeyPath<MarkerDoc, T>, _ fallback: T) -> Binding<T> {
        Binding(
            get: { index.map { draft.markers[$0][keyPath: keyPath] } ?? fallback },
            set: { value in if let i = index { draft.markers[i][keyPath: keyPath] = value } }
        )
    }

    private func actionField<T>(_ keyPath: WritableKeyPath<ActionDoc, T>, _ fallback: T) -> Binding<T> {
        Binding(
            get: { index.map { draft.markers[$0].action[keyPath: keyPath] } ?? fallback },
            set: { value in if let i = index { draft.markers[i].action[keyPath: keyPath] = value } }
        )
    }

    private var kind: Binding<ActionKind> {
        Binding(
            get: { index.map { draft.markers[$0].action.kind } ?? .stop },
            set: { kind in
                guard let i = index else { return }
                draft.markers[i].action.kind = kind
                if kind.needsTarget, draft.markers[i].action.to == nil {
                    draft.markers[i].action.to = TargetDoc(time: 0, marker: nil)
                }
                if kind == .fadeOut, draft.markers[i].action.seconds == nil {
                    draft.markers[i].action.seconds = 2
                }
            }
        )
    }

    /// 0 = a time; otherwise a marker id.
    private var targetChoice: Binding<UInt64> {
        Binding(
            get: { actionField(\.to, nil).wrappedValue?.marker ?? 0 },
            set: { choice in
                actionField(\.to, nil).wrappedValue = choice == 0
                    ? TargetDoc(time: 0, marker: nil)
                    : TargetDoc(time: nil, marker: choice)
            }
        )
    }

    private var targetTime: Binding<Double> {
        Binding(
            get: { actionField(\.to, nil).wrappedValue?.time ?? 0 },
            set: { actionField(\.to, nil).wrappedValue = TargetDoc(time: max($0, 0), marker: nil) }
        )
    }

    private enum LoopMode: Hashable {
        case times, vamp, forever
    }

    private var loopMode: Binding<LoopMode> {
        Binding(
            get: {
                if actionField(\.hold, nil).wrappedValue == true { return .vamp }
                return actionField(\.count, nil).wrappedValue == nil ? .forever : .times
            },
            set: { mode in
                switch mode {
                case .times:
                    actionField(\.hold, nil).wrappedValue = nil
                    if actionField(\.count, nil).wrappedValue == nil {
                        actionField(\.count, nil).wrappedValue = 1
                    }
                case .vamp:
                    actionField(\.hold, nil).wrappedValue = true
                    actionField(\.count, nil).wrappedValue = nil
                case .forever:
                    actionField(\.hold, nil).wrappedValue = nil
                    actionField(\.count, nil).wrappedValue = nil
                }
            }
        )
    }

    private var count: Binding<Int> {
        Binding(
            get: { Int(actionField(\.count, nil).wrappedValue ?? 1) },
            set: { actionField(\.count, nil).wrappedValue = UInt32(clamping: $0) }
        )
    }

    private var fadeSeconds: Binding<Double> {
        Binding(
            get: { actionField(\.seconds, nil).wrappedValue ?? 2 },
            set: { actionField(\.seconds, nil).wrappedValue = max($0, 0) }
        )
    }
}

/// The pad's MIDI trigger, with Learn (live, applies immediately) and
/// Clear (part of the draft).
private struct MidiField: View {
    @EnvironmentObject private var model: AppModel
    @Binding var trigger: MidiTrigger?
    let slot: Int

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "pianokeys")
            if model.learningTarget == "cue:\(slot)" {
                Text("Press a MIDI control…").foregroundStyle(.orange)
                Button("Cancel") { model.cancelLearn() }
            } else {
                Text(trigger?.label ?? "No MIDI")
                    .foregroundStyle(trigger == nil ? Color.secondary : Color.primary)
                Button("Learn") { model.learnMIDI("cue:\(slot)") }
                    .help("Press a MIDI key or pad to fire this pad with it")
                if trigger != nil {
                    Button("Clear") { trigger = nil }
                }
            }
        }
        // A learn completes on the engine side; pick up the result.
        .onChange(of: model.show.cue(at: slot)?.midi) { _, learned in
            if let learned { trigger = learned }
        }
    }
}

/// How the pad behaves when fired: exclusive group, ducking, what a second
/// press does, and which output channels it uses.
private struct PlaybackRow: View {
    @Binding var draft: CueDoc
    let channels: Int

    var body: some View {
        HStack(spacing: 20) {
            HStack(spacing: 6) {
                Text("Group")
                TextField("none", text: group)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 110)
            }
            .help("Pads in the same group (e.g. \"Music\") fade each other out")

            HStack(spacing: 6) {
                Toggle("Duck others", isOn: ducking)
                if ducking.wrappedValue {
                    Slider(value: duckDB, in: -40 ... -3)
                        .frame(width: 100)
                    Text(String(format: "%.0f dB", duckDB.wrappedValue))
                        .font(.caption.monospacedDigit())
                        .frame(width: 44, alignment: .leading)
                }
            }
            .help("Lower everything else on this output while this pad plays")

            Picker("Pressed again", selection: $draft.retrigger) {
                ForEach(Retrigger.allCases, id: \.self) { mode in
                    Text(mode.label).tag(mode)
                }
            }
            .frame(width: 210)
            .help("What pressing the pad does while it plays (a vamp always continues)")

            HStack(spacing: 4) {
                Text("Fade in")
                TextField("", value: $draft.fadeIn, format: .number.precision(.fractionLength(0...2)))
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 44)
                Text("out")
                TextField("", value: fadeOut, format: .number.precision(.fractionLength(0...2)))
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 44)
                Text("s").foregroundStyle(.secondary)
            }
            .help("Fade in each time the pad fires; fade-out length for Fade Out and \"pressed again: fade\"")

            Picker("Channels", selection: route) {
                Text("All").tag("")
                ForEach(routeOptions, id: \.self) { option in
                    Text(option.label).tag("\(option.first):\(option.count)")
                }
                if let current = draft.route, !routeOptions.contains(current) {
                    Text("\(current.label) (not on device)").tag("\(current.first):\(current.count)")
                }
            }
            .frame(width: 200)
            Spacer()
        }
    }

    /// Stereo pairs, then single channels.
    private var routeOptions: [RouteDoc] {
        let pairs = stride(from: 1, to: channels, by: 2).map { RouteDoc(first: $0, count: 2) }
        let singles = (1...max(channels, 1)).map { RouteDoc(first: $0, count: 1) }
        return pairs + singles
    }

    private var fadeOut: Binding<Double> {
        Binding(
            get: { draft.fadeOutSeconds },
            set: { draft.fadeOut = max($0, 0) }
        )
    }

    private var group: Binding<String> {
        Binding(
            get: { draft.group ?? "" },
            set: { draft.group = $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 }
        )
    }

    private var ducking: Binding<Bool> {
        Binding(
            get: { draft.duck != nil },
            set: { draft.duck = $0 ? -12 : nil }
        )
    }

    private var duckDB: Binding<Double> {
        Binding(
            get: { Double(draft.duck ?? -12) },
            set: { draft.duck = Float($0.rounded()) }
        )
    }

    private var route: Binding<String> {
        Binding(
            get: { draft.route.map { "\($0.first):\($0.count)" } ?? "" },
            set: { tag in
                let parts = tag.split(separator: ":").compactMap { Int($0) }
                draft.route = parts.count == 2 ? RouteDoc(first: parts[0], count: parts[1]) : nil
            }
        )
    }
}

/// A playlist pad's songs: order (drag to reorder), shuffle,
/// auto-advance and crossfade.
private struct PlaylistEditor: View {
    @Binding var draft: CueDoc
    let currentItem: Int?
    let onPlay: () -> Void
    let onSkip: () -> Void
    let onStop: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 14) {
                Button(action: addSongs) { Label("Add Songs…", systemImage: "music.note.list") }
                Toggle("Shuffle", isOn: field(\.shuffle, false))
                Toggle("Play on into the next song", isOn: field(\.autoAdvance, true))
                HStack(spacing: 4) {
                    Text("Crossfade")
                    TextField("", value: field(\.crossfade, 2), format: .number.precision(.fractionLength(0...1)))
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 44)
                    Text("s").foregroundStyle(.secondary)
                }
                Spacer()
                Button(action: onPlay) { Image(systemName: "play.fill") }
                    .help("Play")
                Button(action: onSkip) { Image(systemName: "forward.end.fill") }
                    .help("Next song")
                Button(action: onStop) { Image(systemName: "stop.fill") }
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)

            List {
                ForEach(Array(items.enumerated()), id: \.offset) { index, file in
                    HStack {
                        Text("\(index + 1)")
                            .font(.callout.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .frame(width: 28, alignment: .trailing)
                        Image(systemName: currentItem == index ? "speaker.wave.2.fill" : "music.note")
                            .foregroundStyle(currentItem == index ? Color.green : Color.secondary)
                        Text(URL(fileURLWithPath: file).deletingPathExtension().lastPathComponent)
                        Spacer()
                        Button(role: .destructive) {
                            remove(at: index)
                        } label: {
                            Image(systemName: "minus.circle")
                        }
                        .buttonStyle(.borderless)
                        .help("Remove from the playlist")
                    }
                }
                .onMove(perform: move)
            }
            Text("Each press plays the next song; pressing while it plays crossfades to the next (or set \"Pressed again\" to stop or fade). Right-click the pad for Next Song.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 16)
                .padding(.bottom, 8)
        }
    }

    private var items: [String] { draft.playlist?.items ?? [] }

    private func field<T>(_ keyPath: WritableKeyPath<PlaylistDoc, T>, _ fallback: T) -> Binding<T> {
        Binding(
            get: { draft.playlist?[keyPath: keyPath] ?? fallback },
            set: { value in draft.playlist?[keyPath: keyPath] = value }
        )
    }

    private func addSongs() {
        let panel = NSOpenPanel()
        panel.title = "Add songs"
        panel.allowedContentTypes = [.audio]
        panel.allowsMultipleSelection = true
        guard panel.runModal() == .OK else { return }
        draft.playlist?.items.append(contentsOf: panel.urls.map(\.path))
    }

    private func remove(at index: Int) {
        guard draft.playlist?.items.indices.contains(index) == true else { return }
        draft.playlist?.items.remove(at: index)
    }

    private func move(from source: IndexSet, to destination: Int) {
        draft.playlist?.items.move(fromOffsets: source, toOffset: destination)
    }
}
