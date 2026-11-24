import AppKit
import Foundation
import UniformTypeIdentifiers

struct AlertMessage: Identifiable {
    let id = UUID()
    let title: String
    let message: String
}

/// Identifies the pad being edited (for `.sheet(item:)`).
struct EditTarget: Identifiable {
    let id: Int
}

extension UTType {
    static let sportsballShow = UTType(filenameExtension: "sbjshow") ?? .json
}

/// UI state, kept in sync with the Rust engine session. The show document
/// lives in Rust; `show` here is a snapshot refreshed after each change.
@MainActor
final class AppModel: ObservableObject {
    /// Default key for each pad, in grid order, when a sound is assigned.
    static let defaultHotkeys = Array("1234567890qwertyuiopasdfghjklzxcvbnm")
    /// Fade used by Stop All and per-pad "Fade Out".
    static let stopFadeSeconds = 0.5

    @Published private(set) var show = ShowDoc()
    @Published private(set) var cueStates: [CueStatus] = []
    @Published private(set) var status: SessionStatus?
    @Published private(set) var devices: [AudioDevice] = []
    @Published private(set) var busySlots: Set<Int> = []
    @Published private(set) var isLoadingShow = false
    @Published private(set) var notices: [String] = []
    @Published var alert: AlertMessage?
    @Published var editing: EditTarget?
    @Published var showingOutputs = false
    @Published var showingSettings = false
    @Published var showingControls = false
    @Published var showingPreflight = false
    @Published private(set) var preflight: PreflightReport?
    @Published var midiEnabled: Bool = UserDefaults.standard.object(forKey: "midiEnabled") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(midiEnabled, forKey: "midiEnabled")
            applyMIDI()
        }
    }
    /// 0 = OSC off.
    @Published var oscPort: Int = UserDefaults.standard.integer(forKey: "oscPort") {
        didSet {
            UserDefaults.standard.set(oscPort, forKey: "oscPort")
            applyOSC()
        }
    }
    @Published var masterVolume: Double = 1.0 {
        didSet { report { try session.setMasterGain(Self.gain(forSlider: masterVolume)) } }
    }

    let session: EngineSession
    private var pollTask: Task<Void, Never>?
    private var activity: NSObjectProtocol?
    private var tick = 0
    private var lastSave = Date.distantPast
    /// Durations and waveforms of loaded files, for the editor.
    @Published private(set) var durations: [String: Double] = [:]
    @Published private(set) var waveforms: [String: [Float]] = [:]

    static let waveformBuckets = 4000
    private static let lastShowKey = "lastShowPath"

    init() {
        guard Engine.abiMatches else {
            fatalError("Engine header and library are out of sync; rebuild the Rust engine.")
        }
        do {
            session = try EngineSession()
        } catch {
            fatalError("Could not create engine session: \(error.localizedDescription)")
        }

        // Keep macOS from throttling us (App Nap) during a live show.
        activity = ProcessInfo.processInfo.beginActivity(
            options: [.userInitiated, .latencyCritical],
            reason: "Live audio playback"
        )

        refreshDevices()
        session.openOutputs()
        refreshShow()
        applyMIDI()
        applyOSC()

        if let screenshots = ScreenshotDirector.fromArguments() {
            // Demo / documentation mode: open the given show, capture the
            // main screens, quit.
            openShow(at: screenshots.show)
            screenshots.run(model: self)
        } else if let path = UserDefaults.standard.string(forKey: Self.lastShowKey),
                  FileManager.default.fileExists(atPath: path) {
            openShow(at: URL(fileURLWithPath: path))
        }

        pollTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                self?.onTick()
                try? await Task.sleep(nanoseconds: 33_000_000)  // ~30 Hz
            }
        }
    }

    /// Slider position (0...1) to linear gain, with a perceptual curve.
    static func gain(forSlider value: Double) -> Float {
        Float(value * value)
    }

    var windowTitle: String {
        let name = show.name.isEmpty ? "Untitled Show" : show.name
        return (status?.dirty ?? false) ? "\(name) — Edited" : name
    }

    /// The bank shown in the grid (MIDI/OSC can change it too).
    var currentBank: Int {
        min(status?.bank ?? 0, max(show.banks.count - 1, 0))
    }

    func selectBank(_ bank: Int) {
        report { try session.setBank(bank) }
        refreshStatus()
    }

    func state(of slot: Int) -> CueStatus {
        cueStates.indices.contains(slot) ? cueStates[slot] : CueStatus()
    }

    func problems(for slot: Int) -> [String] {
        (status?.problems ?? []).filter { $0.slot == slot }.map(\.message)
    }

    // MARK: - Polling

    private func onTick() {
        session.poll()
        let states = session.cueStates(count: show.slots * max(show.banks.count, 1))
        if states != cueStates {
            cueStates = states
        }
        tick += 1
        if tick % 3 == 0 {  // ~10 Hz: bank changes and MIDI learn feel live
            refreshStatus()
        }
        if tick % 30 == 0 {  // ~1 Hz
            autosaveIfNeeded()
        }
    }

    private func refreshStatus() {
        guard let new = session.status() else { return }
        if !new.notices.isEmpty {
            notices.append(contentsOf: new.notices)
            let count = new.notices.count
            Task { @MainActor [weak self] in
                try? await Task.sleep(nanoseconds: 8_000_000_000)
                self?.notices.removeFirst(min(count, self?.notices.count ?? 0))
            }
        }
        var current = new
        current.notices = []
        // A finished MIDI learn changed the show on the engine side.
        let finishedLearning = status?.learning != nil && current.learning == nil
        if current != status {
            status = current
        }
        if finishedLearning {
            refreshShow()
        }
    }

    func refreshShow() {
        if let doc = session.show(), doc != show {
            show = doc
        }
        refreshStatus()
    }

    func refreshDevices() {
        devices = session.devices()
    }

    // MARK: - Shows

    func newShow() {
        if isAutosaveFile(status?.path), !show.cues.isEmpty {
            let confirm = NSAlert()
            confirm.messageText = "Discard the untitled show?"
            confirm.informativeText = "It has never been saved. Use Save As to keep it."
            confirm.addButton(withTitle: "Discard")
            confirm.addButton(withTitle: "Cancel")
            guard confirm.runModal() == .alertFirstButtonReturn else { return }
        }
        report { try session.newShow() }
        UserDefaults.standard.removeObject(forKey: Self.lastShowKey)
        refreshShow()
    }

    func openShowPanel() {
        let panel = NSOpenPanel()
        panel.title = "Open Show"
        panel.allowedContentTypes = [.sportsballShow]
        panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            openShow(at: url)
        }
    }

    /// Opens a show in the background (decoding its audio can take a while).
    func openShow(at url: URL) {
        isLoadingShow = true
        let session = self.session
        Task.detached(priority: .userInitiated) {
            var failure: String?
            do {
                try session.openShow(at: url)
            } catch {
                failure = error.localizedDescription
            }
            let message = failure
            await MainActor.run {
                self.isLoadingShow = false
                if let message {
                    self.alert = AlertMessage(
                        title: "Could not open \(url.lastPathComponent)",
                        message: message
                    )
                } else {
                    UserDefaults.standard.set(url.path, forKey: Self.lastShowKey)
                    self.durations = [:]
                    self.waveforms = [:]
                    self.lastSave = Date()
                }
                self.refreshShow()
            }
        }
    }

    func save() {
        if status?.path == nil || isAutosaveFile(status?.path) {
            saveAs()
        } else {
            report { try session.saveShow(to: nil) }
            lastSave = Date()
            refreshStatus()
        }
    }

    func saveAs() {
        let panel = NSSavePanel()
        panel.title = "Save Show"
        panel.allowedContentTypes = [.sportsballShow]
        panel.nameFieldStringValue = show.name.isEmpty ? "Untitled Show" : show.name
        guard panel.runModal() == .OK, let url = panel.url else { return }
        report { try session.saveShow(to: url) }
        UserDefaults.standard.set(url.path, forKey: Self.lastShowKey)
        lastSave = Date()
        refreshStatus()
    }

    /// Where untitled shows are kept until saved, so nothing is lost.
    private static var autosaveURL: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let dir = base.appendingPathComponent("Sportsball Jukebox", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("Untitled Show.sbjshow")
    }

    private func isAutosaveFile(_ path: String?) -> Bool {
        path == Self.autosaveURL.path
    }

    /// Saves changes a couple of seconds after they are made: to the show's
    /// own file, or to the autosave file for untitled shows.
    private func autosaveIfNeeded() {
        guard let status, status.dirty, Date().timeIntervalSince(lastSave) > 2 else { return }
        lastSave = Date()
        let url: URL? = status.path == nil ? Self.autosaveURL : nil
        do {
            try session.saveShow(to: url)
            if let url {
                UserDefaults.standard.set(url.path, forKey: Self.lastShowKey)
            }
        } catch {
            // Surface once; retried on the next change.
            alert = AlertMessage(title: "Autosave failed", message: error.localizedDescription)
        }
    }

    func applySettings(name: String, slots: Int, columns: Int, banks: [String], stopAllFade: Double) {
        report {
            try session.setShowSettings(
                name: name, slots: slots, columns: columns, banks: banks, stopAllFade: stopAllFade
            )
        }
        refreshShow()
    }

    // MARK: - MIDI, OSC, pre-game check

    private func applyMIDI() {
        do {
            try session.enableMIDI(midiEnabled)
        } catch {
            alert = AlertMessage(title: "MIDI", message: error.localizedDescription)
        }
        refreshStatus()
    }

    private func applyOSC() {
        do {
            try session.setOSCPort(oscPort)
        } catch {
            alert = AlertMessage(title: "OSC", message: error.localizedDescription)
        }
        refreshStatus()
    }

    func rescanMIDI() {
        applyMIDI()
    }

    /// Starts MIDI learn for "cue:<slot>", "stop_all", "next_bank" or
    /// "prev_bank"; the next MIDI message received is assigned to it.
    func learnMIDI(_ target: String) {
        if !midiEnabled {
            midiEnabled = true
        }
        report { try session.learnMIDI(target: target) }
        refreshStatus()
    }

    func cancelLearn() {
        report { try session.learnMIDI(target: nil) }
        refreshStatus()
    }

    func clearMIDI(_ target: String) {
        report { try session.clearMIDI(target: target) }
        refreshShow()
    }

    var learningTarget: String? { status?.learning }

    func runPreflight() {
        preflight = session.preflight()
    }


    func applyOutputs(_ outputs: [OutputDoc]) {
        report { try session.setOutputs(outputs) }
        refreshShow()
    }

    // MARK: - Cues

    /// Clicking a pad: fire it if it has a cue, otherwise choose a sound.
    func trigger(_ slot: Int) {
        if show.cue(at: slot) != nil {
            report { try session.fire(slot: slot) }
        } else if !busySlots.contains(slot) {
            chooseSound(for: slot)
        }
    }

    func chooseSound(for slot: Int) {
        let panel = NSOpenPanel()
        panel.title = "Choose a sound for pad \(slot + 1)"
        panel.message = "Choose several files to make a playlist pad."
        panel.allowedContentTypes = [.audio]
        panel.allowsMultipleSelection = true
        if panel.runModal() == .OK {
            assignSounds(urls: panel.urls, to: slot)
        }
    }

    /// One file: a normal pad. Several: a playlist pad.
    func assignSounds(urls: [URL], to slot: Int) {
        guard let first = urls.first else { return }
        guard urls.count > 1 else {
            assignSound(url: first, to: slot)
            return
        }
        var cue = show.cue(at: slot) ?? CueDoc(slot: slot)
        if cue.playlist == nil {
            cue.title = "Playlist"
        }
        cue.clips = []
        cue.markers = []
        cue.playlist = PlaylistDoc(items: urls.map(\.path))
        if cue.hotkey == nil {
            cue.hotkey = freeHotkey(preferring: slot)
        }
        updateCue(cue)
    }

    /// Playlist pad: crossfade to the next song.
    func skip(_ slot: Int) {
        report { try session.skip(slot: slot) }
    }

    /// Puts a file on a pad. An existing cue keeps its settings (key,
    /// output, volume) but gets the new sound.
    func assignSound(url: URL, to slot: Int) {
        let title = url.deletingPathExtension().lastPathComponent
        var cue = show.cue(at: slot) ?? CueDoc(slot: slot, title: title, file: url.path)
        cue.title = title
        cue.clips = [ClipDoc(file: url.path)]
        cue.markers = []
        cue.playlist = nil
        if cue.hotkey == nil {
            cue.hotkey = freeHotkey(preferring: slot)
        }
        updateCue(cue)
    }

    /// The pad's default key (by its position in its bank) if unused in
    /// that bank, else the first unused one. Keys only act in the visible
    /// bank, so banks can reuse them.
    private func freeHotkey(preferring slot: Int) -> String? {
        let perBank = max(show.slots, 1)
        let bank = slot / perBank
        let used = Set(show.cues.filter { $0.slot / perBank == bank }.compactMap(\.hotkey))
        let keys = Self.defaultHotkeys.map(String.init)
        let position = slot % perBank
        if keys.indices.contains(position), !used.contains(keys[position]) {
            return keys[position]
        }
        return keys.first { !used.contains($0) }
    }

    /// Applies a cue in the background (it may need to decode audio).
    func updateCue(_ cue: CueDoc) {
        busySlots.insert(cue.slot)
        let session = self.session
        Task.detached(priority: .userInitiated) {
            var failure: String?
            do {
                try session.setCue(cue)
            } catch {
                failure = error.localizedDescription
            }
            let message = failure
            await MainActor.run {
                self.busySlots.remove(cue.slot)
                if let message {
                    self.alert = AlertMessage(title: "Could not update pad", message: message)
                }
                self.refreshShow()
            }
        }
    }

    func clear(_ slot: Int) {
        report { try session.removeCue(slot: slot) }
        refreshShow()
    }

    func move(from: Int, to: Int) {
        guard from != to else { return }
        report { try session.moveCue(from: from, to: to) }
        refreshShow()
    }

    func setLoop(_ slot: Int, _ looping: Bool) {
        guard var cue = show.cue(at: slot) else { return }
        cue.loop = looping
        updateCue(cue)
    }

    func stop(_ slot: Int, fade: Bool) {
        let seconds = fade ? (show.cue(at: slot)?.fadeOutSeconds ?? Self.stopFadeSeconds) : 0
        report { try session.stop(slot: slot, fadeSeconds: seconds) }
    }

    func stopAll() {
        report { try session.stopAll(fadeSeconds: show.stopAllFade) }
    }

    func play(_ slot: Int, from seconds: Double) {
        report { try session.play(slot: slot, from: seconds) }
    }

    // MARK: - Audio info for the editor

    /// Loads a file's duration and waveform in the background; they
    /// appear in `durations` / `waveforms` when ready.
    func loadAudioInfo(for file: String) {
        if durations[file] != nil {
            return
        }
        let session = self.session
        let buckets = Self.waveformBuckets
        Task.detached(priority: .userInitiated) {
            let duration = try? session.loadSound(file: file)
            let peaks = session.waveform(file: file, buckets: buckets)
            await MainActor.run {
                if let duration {
                    self.durations[file] = duration
                }
                if let peaks {
                    self.waveforms[file] = peaks
                }
            }
        }
    }

    func duration(of file: String) -> Double? {
        durations[file]
    }

    func waveform(of file: String) -> [Float]? {
        waveforms[file]
    }

    // MARK: - Errors

    /// Runs an engine call, surfacing any failure as an alert.
    private func report(_ action: () throws -> Void) {
        do {
            try action()
        } catch {
            alert = AlertMessage(title: "Something went wrong", message: error.localizedDescription)
        }
    }
}
