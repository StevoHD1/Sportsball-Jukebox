import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.undoManager) private var undoManager

    var body: some View {
        VStack(spacing: 0) {
            HeaderBar()
            Divider()
            OutputBanners()
            NoticeToasts()
            BankBar()
            if model.showingNowPlaying && model.nowPlayingOnTop {
                Divider()
                NowPlayingStrip()
                Divider()
            }
            HStack(spacing: 0) {
                if model.showingLibrary {
                    LibraryView(width: model.libraryWidth)
                        .frame(maxHeight: .infinity)
                    ResizeHandle(width: $model.libraryWidth, range: 220...640, direction: 1)
                } else if model.showingRundown {
                    RundownView(width: model.rundownWidth)
                        .frame(maxHeight: .infinity)
                    ResizeHandle(width: $model.rundownWidth, range: 220...560, direction: 1)
                }
                PadArea()
                    // Take all the width the panels leave (without this the
                    // row shrinks to its content and the pads get squeezed).
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                if model.showingNowPlaying && !model.nowPlayingOnTop {
                    ResizeHandle(width: $model.nowPlayingWidth, range: 220...520, direction: -1)
                    NowPlayingView(width: model.nowPlayingWidth)
                        .frame(maxHeight: .infinity)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            Divider()
            ControlFooter()
        }
        .frame(minWidth: 820, minHeight: 480)
        .navigationTitle(model.windowTitle)
        .overlay(alignment: .bottomTrailing) {
            PreviewPlayer()
                .padding(.trailing, 16)
                .padding(.bottom, 40)
        }
        .onAppear { model.undoManager = undoManager }
        .onChange(of: undoManager) { _, manager in model.undoManager = manager }
        .overlay {
            if let message = model.busyMessage {
                ProgressView(message)
                    .padding(24)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            } else if model.isLoadingShow {
                ProgressView("Loading show…")
                    .padding(24)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .alert(item: $model.alert) { alert in
            Alert(title: Text(alert.title), message: Text(alert.message))
        }
        .sheet(item: $model.editing) { target in
            CueEditorView(slot: target.id)
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingOutputs) {
            OutputsView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingSettings) {
            ShowSettingsView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingControls) {
            ControlsView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingPreflight) {
            PreflightView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingColours) {
            ColourPresetsView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingLog) {
            GameLogView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showingEvents) {
            EventLogView()
                .environmentObject(model)
        }
    }
}

/// The pad grid: fitted to the window's height, or at a fixed pad size
/// with scrolling.
private struct PadArea: View {
    @EnvironmentObject private var model: AppModel

    private static let padding: CGFloat = 16
    private static let spacing: CGFloat = 10

    var body: some View {
        GeometryReader { viewport in
            ScrollView {
                PadGrid(padHeight: padHeight(for: viewport.size.height))
                    .padding(Self.padding)
            }
        }
    }

    private func padHeight(for height: CGFloat) -> CGFloat {
        guard model.fitPadsToWindow else { return CGFloat(model.padHeight) }
        let columns = max(model.show.columns, 1)
        let rows = max((model.show.slots + columns - 1) / columns, 1)
        let available = height - 2 * Self.padding - Self.spacing * CGFloat(rows - 1)
        // Below this, pads scroll rather than shrink further.
        return max(available / CGFloat(rows), 44)
    }
}

private struct PadGrid: View {
    @EnvironmentObject private var model: AppModel
    let padHeight: CGFloat

    var body: some View {
        let columns = Array(
            repeating: GridItem(.flexible(), spacing: 10),
            count: max(model.show.columns, 1)
        )
        LazyVGrid(columns: columns, spacing: 10) {
            ForEach(Array(model.show.padRange(bank: model.currentBank)), id: \.self) { slot in
                CueButton(slot: slot, height: padHeight)
            }
        }
    }
}

private struct HeaderBar: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 14) {
            Button {
                model.showingOutputs = true
            } label: {
                Label("Outputs", systemImage: "hifispeaker.2")
            }
            .help("Choose audio interfaces and backups")

            ForEach(Array((model.status?.outputs ?? []).enumerated()), id: \.offset) { _, output in
                OutputBadge(output: output)
            }

            Spacer()

            Button {
                model.showingLibrary.toggle()
            } label: {
                Label("Library", systemImage: "music.note.list")
            }
            .labelStyle(.iconOnly)
            .help("Show or hide the audio library (⌥⌘L)")

            Button {
                model.showingRundown.toggle()
            } label: {
                Label("Rundown", systemImage: "list.number")
            }
            .labelStyle(.iconOnly)
            .help("Show or hide the game rundown (GO with Space)")

            Button {
                model.showingNowPlaying.toggle()
            } label: {
                Label("Now Playing", systemImage: "sidebar.right")
            }
            .labelStyle(.iconOnly)
            .help("Show or hide the Now Playing panel")

            Button {
                model.runPreflight()
                model.showingPreflight = true
            } label: {
                Label("Pre-game Check", systemImage: "checklist")
            }
            .labelStyle(.iconOnly)
            .help("Check devices, files, keys and MIDI before the game")

            Button {
                model.showingLog = true
            } label: {
                Label("Game Log", systemImage: "list.bullet.rectangle")
            }
            .labelStyle(.iconOnly)
            .help("What played, for licensing and sponsor reports")

            Button {
                model.showingEvents = true
            } label: {
                Label("Event Log", systemImage: "stethoscope")
            }
            .labelStyle(.iconOnly)
            .help("Glitches, device problems and warnings, with times (⇧⌘E)")

            Button {
                model.showingControls = true
            } label: {
                Label("Control", systemImage: "pianokeys")
            }
            .labelStyle(.iconOnly)
            .help("External control: MIDI, OSC, Stream Deck, phone/tablet remote")

            Image(systemName: "speaker.wave.2.fill")
                .foregroundStyle(.secondary)
            Slider(value: $model.masterVolume, in: 0...1)
                .frame(width: 160)
                .help("Master volume")

            Button {
                model.stopLast(fade: true)
            } label: {
                Label("Fade Last", systemImage: "speaker.wave.1")
            }
            .keyboardShortcut(.escape, modifiers: [.shift])
            .disabled(model.lastPad == nil)
            .help(model.lastPad.map { "Fade out \"\($0.title)\", the last pad fired (⇧Esc)" }
                ?? "Fade out the last pad fired (⇧Esc)")
            Button {
                model.stopLast(fade: false)
            } label: {
                Label("Stop Last", systemImage: "stop")
            }
            .labelStyle(.iconOnly)
            .keyboardShortcut(.escape, modifiers: [.option])
            .disabled(model.lastPad == nil)
            .help(model.lastPad.map { "Stop \"\($0.title)\", the last pad fired (⌥Esc)" }
                ?? "Stop the last pad fired (⌥Esc)")

            Button(role: .destructive) {
                model.stopAll()
            } label: {
                Label("Stop All", systemImage: "stop.fill")
                    .font(.headline)
                    .padding(.horizontal, 6)
            }
            .buttonStyle(.borderedProminent)
            .tint(.red)
            .keyboardShortcut(.escape, modifiers: [])
            .help("Fade out everything (Esc)")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
    }
}

private struct OutputBadge: View {
    @EnvironmentObject private var model: AppModel
    let output: OutputState

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 9, height: 9)
            Text(text)
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .contentShape(Rectangle())
        .onTapGesture { model.showingEvents = true }
        .help(help)
    }

    private var help: String {
        let base = output.error ?? (output.deviceName ?? "")
        return output.xruns > 0 ? "\(base)\nClick to see when the glitches happened" : base
    }

    private var color: Color {
        switch output.status {
        case "running": return output.xruns > 0 ? .orange : .green
        case "on_backup": return .orange
        default: return .red
        }
    }

    private var text: String {
        guard output.isUp else { return "\(output.name): offline" }
        var text = "\(output.name): \(output.deviceName ?? "?")"
        if output.status == "on_backup" {
            text += " (backup)"
        }
        if output.xruns > 0 {
            text += " · \(output.xruns) glitch\(output.xruns == 1 ? "" : "es")"
        }
        return text
    }
}

/// Persistent red banners for outputs with no device.
private struct OutputBanners: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let down = (model.status?.outputs ?? []).filter { !$0.isUp }
        ForEach(Array(down.enumerated()), id: \.offset) { _, output in
            Label(
                "Output \"\(output.name)\" has no audio device. Its pads cannot play. Retrying…",
                systemImage: "exclamationmark.triangle.fill"
            )
            .font(.headline)
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity)
            .padding(8)
            .background(Color.red)
        }
    }
}

/// Short-lived messages such as "switched to backup device".
private struct NoticeToasts: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ForEach(Array(model.notices.enumerated()), id: \.offset) { _, notice in
            Label(notice, systemImage: "info.circle.fill")
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                .background(Color.orange.opacity(0.25))
        }
    }
}

/// Tabs for the show's banks of pads.
private struct BankBar: View {
    @EnvironmentObject private var model: AppModel

    private var multiplay: Binding<Bool> {
        Binding(
            get: { model.isMultiplay(bank: model.currentBank) },
            set: { model.setMultiplay(bank: model.currentBank, $0) }
        )
    }

    /// Moving the size slider switches off Fit.
    private var padHeight: Binding<Double> {
        Binding(
            get: { model.padHeight },
            set: {
                model.fitPadsToWindow = false
                model.padHeight = $0
            }
        )
    }

    var body: some View {
        HStack(spacing: 6) {
            ForEach(Array(model.show.banks.enumerated()), id: \.offset) { index, name in
                Button {
                    model.selectBank(index)
                } label: {
                    Text(index < 9 ? "\(name)  ⌘\(index + 1)" : name)
                        .font(.callout.weight(index == model.currentBank ? .semibold : .regular))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 5)
                        .background(
                            index == model.currentBank ? Color.accentColor : Color.secondary.opacity(0.15),
                            in: Capsule()
                        )
                        .foregroundStyle(index == model.currentBank ? Color.white : Color.primary)
                }
                .buttonStyle(.plain)
                .help("Pad keys apply to the bank shown")
                .accessibilityLabel("Bank \(name)")
                .accessibilityAddTraits(index == model.currentBank ? .isSelected : [])
            }
            Spacer()
            if model.playedOnThisBank > 0 {
                Button {
                    model.resetPlayed(bank: model.currentBank)
                } label: {
                    Label("Reset Played (\(model.playedOnThisBank))", systemImage: "arrow.counterclockwise")
                }
                .controlSize(.small)
                .help("Unmark this page's pads as played")
            }
            Toggle("Multiplay", isOn: multiplay)
                .toggleStyle(.switch)
                .controlSize(.small)
                .help("On: this bank's pads play over each other. Off: firing a pad fades out the bank's other pads.")
            Divider().frame(height: 18)
            Toggle("Fit", isOn: $model.fitPadsToWindow)
                .toggleStyle(.switch)
                .controlSize(.small)
                .help("Size the pads to fit the window (otherwise they keep their size and scroll)")
            Image(systemName: "square.grid.3x3")
                .foregroundStyle(.secondary)
            Slider(value: padHeight, in: AppModel.padHeights)
                .frame(width: 110)
                .controlSize(.small)
                .help("Pad size (⌘+ / ⌘−)")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }
}

/// Bottom line: MIDI / OSC / remote state and the last control message received.
private struct ControlFooter: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        HStack(spacing: 14) {
            if let learning = model.learningTarget {
                Label("Waiting for MIDI to learn \(describe(learning))…", systemImage: "dot.radiowaves.left.and.right")
                    .foregroundStyle(.orange)
                Button("Cancel") { model.cancelLearn() }
            } else if let last = model.status?.lastControl {
                Label(last, systemImage: "dot.radiowaves.left.and.right")
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Text(midiText)
            Text(oscText)
            if let port = model.status?.remotePort {
                Text("Remote: port \(String(port))")
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(.bar)
    }

    private var midiText: String {
        guard let inputs = model.status?.midiInputs else { return "MIDI off" }
        switch inputs.count {
        case 0: return "MIDI: no devices"
        case 1: return "MIDI: \(inputs[0])"
        default: return "MIDI: \(inputs.count) devices"
        }
    }

    private var oscText: String {
        model.status?.oscPort.map { "OSC: port \($0)" } ?? "OSC off"
    }

    private func describe(_ target: String) -> String {
        switch target {
        case "stop_all": return "Stop All"
        case "next_bank": return "Next Bank"
        case "prev_bank": return "Previous Bank"
        case "rundown_go": return "Rundown GO"
        case "fade_last": return "Fade Last"
        default:
            let slot = Int(target.dropFirst(4)) ?? 0
            return model.show.cue(at: slot).map { "\"\($0.title)\"" } ?? "pad"
        }
    }
}
