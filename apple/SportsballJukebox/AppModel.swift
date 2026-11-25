import AppKit
import Foundation
import UniformTypeIdentifiers

struct NowPlayingItem: Identifiable {
    let cue: CueDoc
    let state: CueStatus

    var id: Int { cue.slot }
}

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
    @Published var showingLog = false
    @Published var showingRundown: Bool = UserDefaults.standard.bool(forKey: "showRundown") {
        didSet {
            UserDefaults.standard.set(showingRundown, forKey: "showRundown")
            if showingRundown { showingLibrary = false }
        }
    }
    /// The audio library sidebar (shares the left side with the rundown).
    @Published var showingLibrary: Bool = UserDefaults.standard.bool(forKey: "showLibrary") {
        didSet {
            UserDefaults.standard.set(showingLibrary, forKey: "showLibrary")
            if showingLibrary { showingRundown = false }
        }
    }
    @Published private(set) var library: [LibraryEntry] = []
    /// Pad keys and Space are ignored while typing (e.g. in the library
    /// search field).
    @Published var suspendHotkeys = false
    /// From Copy Style, for Paste Style.
    @Published private(set) var copiedStyle: PadStyle?
    /// Pad pictures by stored path (not published: filled while drawing).
    private var imageCache: [String: NSImage] = [:]
    private var missingImages: Set<String> = []
    @Published var showingNowPlaying: Bool = UserDefaults.standard.object(forKey: "showNowPlaying") as? Bool ?? true {
        didSet { UserDefaults.standard.set(showingNowPlaying, forKey: "showNowPlaying") }
    }
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
    /// The phone/tablet remote page (off by default: it opens a port).
    @Published var remoteEnabled: Bool = UserDefaults.standard.bool(forKey: "remoteEnabled") {
        didSet {
            UserDefaults.standard.set(remoteEnabled, forKey: "remoteEnabled")
            applyRemote()
        }
    }
    @Published var remotePort: Int = {
        let saved = UserDefaults.standard.integer(forKey: "remotePort")
        return saved == 0 ? 8765 : saved
    }() {
        didSet {
            UserDefaults.standard.set(remotePort, forKey: "remotePort")
            applyRemote()
        }
    }
    @Published private(set) var remotePin: String = AppModel.savedOrNewPin()
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
    /// Loudness, peak and silence per file, for the editor.
    @Published private(set) var soundInfos: [String: SoundInfo] = [:]

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
        session.setLogDirectory(Self.logDirectory)
        session.openOutputs()
        refreshShow()
        applyMIDI()
        applyOSC()
        applyRemote()

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

    /// Re-reads the show after a change. `undo` names the change as an
    /// undoable step (Edit → Undo <name>) if it altered the show.
    func refreshShow(undo label: String = "Change") {
        if !applyingUndo, session.recordUndo(label) {
            registerUndo(label)
        }
        if let doc = session.show(), doc != show {
            show = doc
        }
        let entries = session.library()
        if entries != library {
            library = entries
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
        clearUndo()
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
                    self.imageCache = [:]
                    self.missingImages = []
                    self.lastSave = Date()
                    self.clearUndo()
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

    func applySettings(
        name: String, slots: Int, columns: Int, banks: [String], stopAllFade: Double,
        levelMatch: Double?
    ) {
        report {
            try session.setShowSettings(
                name: name, slots: slots, columns: columns, banks: banks, stopAllFade: stopAllFade,
                levelMatch: levelMatch
            )
        }
        // The gain level matching would apply depends on the target.
        for file in soundInfos.keys {
            soundInfos[file] = session.soundInfo(file: file)
        }
        refreshShow(undo: "Show Settings")
    }

    // MARK: - Layout (per Mac, not saved in the show)

    /// Pads fill the window's height (true), or have a fixed height and
    /// scroll.
    @Published var fitPadsToWindow: Bool = UserDefaults.standard.object(forKey: "fitPads") as? Bool ?? false {
        didSet { UserDefaults.standard.set(fitPadsToWindow, forKey: "fitPads") }
    }
    /// Pad height when not fitting the window.
    @Published var padHeight: Double = UserDefaults.standard.object(forKey: "padHeight") as? Double ?? 100 {
        didSet { UserDefaults.standard.set(padHeight, forKey: "padHeight") }
    }
    static let padHeights: ClosedRange<Double> = 50...240
    /// Now Playing as a strip across the top (true) or a column on the
    /// right.
    @Published var nowPlayingOnTop: Bool = UserDefaults.standard.object(forKey: "nowPlayingTop") as? Bool ?? true {
        didSet { UserDefaults.standard.set(nowPlayingOnTop, forKey: "nowPlayingTop") }
    }
    @Published var libraryWidth: Double = UserDefaults.standard.object(forKey: "libraryWidth") as? Double ?? 290 {
        didSet { UserDefaults.standard.set(libraryWidth, forKey: "libraryWidth") }
    }
    @Published var rundownWidth: Double = UserDefaults.standard.object(forKey: "rundownWidth") as? Double ?? 270 {
        didSet { UserDefaults.standard.set(rundownWidth, forKey: "rundownWidth") }
    }
    @Published var nowPlayingWidth: Double = UserDefaults.standard.object(forKey: "nowPlayingWidth") as? Double ?? 280 {
        didSet { UserDefaults.standard.set(nowPlayingWidth, forKey: "nowPlayingWidth") }
    }

    func changePadSize(by step: Double) {
        fitPadsToWindow = false
        padHeight = min(max(padHeight + step, Self.padHeights.lowerBound), Self.padHeights.upperBound)
    }

    // MARK: - Undo

    /// The window's undo manager (set by ContentView), so Edit → Undo and
    /// ⌘Z work as on any Mac app, with "Undo Paste Style" and so on.
    weak var undoManager: UndoManager?
    private var applyingUndo = false

    private func registerUndo(_ label: String) {
        guard let undoManager else { return }
        undoManager.registerUndo(withTarget: self) { model in
            MainActor.assumeIsolated { model.undoStep(label) }
        }
        undoManager.setActionName(label)
    }

    private func undoStep(_ label: String) {
        applyingUndo = true
        report { try session.undo() }
        refreshShow()
        applyingUndo = false
        // Registered while undoing, so it becomes the redo.
        undoManager?.registerUndo(withTarget: self) { model in
            MainActor.assumeIsolated { model.redoStep(label) }
        }
        undoManager?.setActionName(label)
    }

    private func redoStep(_ label: String) {
        applyingUndo = true
        report { try session.redo() }
        refreshShow()
        applyingUndo = false
        registerUndo(label)
    }

    /// A different show: nothing from the old one can be undone.
    private func clearUndo() {
        undoManager?.removeAllActions()
    }

    // MARK: - Save With Audio

    /// Shown over the window while something slow runs (e.g. copying).
    @Published private(set) var busyMessage: String?

    /// Saves the show into a new folder together with copies of every file
    /// it uses (Audio and Images folders), so the folder can be moved to
    /// another Mac or a USB stick.
    func saveWithAudio() {
        let panel = NSSavePanel()
        panel.title = "Save With Audio"
        panel.message = "Creates a folder with the show and copies of all its audio and pictures."
        panel.nameFieldStringValue = show.name.isEmpty ? "Untitled Show" : show.name
        panel.allowedContentTypes = [.sportsballShow]
        guard panel.runModal() == .OK, let chosen = panel.url else { return }
        let folder = chosen.deletingPathExtension()
        let showURL = folder.appendingPathComponent(chosen.lastPathComponent)
        busyMessage = "Copying audio…"
        let session = self.session
        Task.detached(priority: .userInitiated) {
            var failure: String?
            do {
                try session.saveWithAudio(to: showURL)
            } catch {
                failure = error.localizedDescription
            }
            let message = failure
            await MainActor.run {
                self.busyMessage = nil
                if let message {
                    self.alert = AlertMessage(title: "Could not save with audio", message: message)
                } else {
                    UserDefaults.standard.set(showURL.path, forKey: Self.lastShowKey)
                    self.lastSave = Date()
                    self.clearUndo()
                    self.imageCache = [:]
                    self.missingImages = []
                    NSWorkspace.shared.activateFileViewerSelecting([showURL])
                }
                self.refreshShow()
            }
        }
    }

    // MARK: - Live transport

    /// Scrubbing in Now Playing.
    func seek(_ slot: Int, to seconds: Double) {
        report { try session.seekCue(slot: slot, seconds: seconds) }
    }

    /// Scrubbing in the preview player.
    func seekPreview(to seconds: Double) {
        report { try session.seekPreview(seconds: seconds) }
        refreshStatus()
    }

    /// The most recently fired pad still playing.
    var lastPad: CueDoc? { status?.last.flatMap { show.cue(at: $0) } }

    func stopLast(fade: Bool) {
        report { try session.stopLast(fade: fade) }
        refreshStatus()
    }

    func isMultiplay(bank: Int) -> Bool {
        !(status?.singlePlay ?? show.singlePlay).contains(bank)
    }

    func setMultiplay(bank: Int, _ on: Bool) {
        report { try session.setBankMultiplay(bank, on) }
        refreshShow(undo: "Multiplay")
    }

    // MARK: - Rundown

    /// The step GO fires next (nil when finished).
    var standby: Int? { status?.standby }

    func rundownGo() {
        report { try session.rundownGo() }
        refreshStatus()
    }

    func setStandby(_ index: Int?) {
        session.setStandby(index)
        refreshStatus()
    }

    func stepStandby(forward: Bool) {
        session.stepStandby(forward: forward)
        refreshStatus()
    }

    func setRundown(_ items: [RundownItem]) {
        report { try session.setRundown(items) }
        refreshShow(undo: "Rundown Change")
    }

    private var nextRundownID: UInt64 {
        (show.rundown.map(\.id).max() ?? 0) + 1
    }

    /// Adds a pad (or, with `pad` nil, a note) after the standby step, or
    /// at the end.
    func addToRundown(pad: Int?, text: String = "") {
        var items = show.rundown
        items.append(RundownItem(id: nextRundownID, pad: pad, text: text))
        setRundown(items)
        showingRundown = true
    }

    func updateRundownItem(_ item: RundownItem) {
        var items = show.rundown
        guard let i = items.firstIndex(where: { $0.id == item.id }) else { return }
        items[i] = item
        setRundown(items)
    }

    func removeRundownItems(at offsets: IndexSet) {
        var items = show.rundown
        items.remove(atOffsets: offsets)
        setRundown(items)
    }

    func moveRundownItems(from: IndexSet, to: Int) {
        var items = show.rundown
        items.move(fromOffsets: from, toOffset: to)
        setRundown(items)
    }

    /// What a step shows: its text, else its pad's title.
    func rundownLabel(_ item: RundownItem) -> String {
        if !item.text.isEmpty { return item.text }
        guard let pad = item.pad else { return "Note" }
        guard let cue = show.cue(at: pad) else { return "Empty pad" }
        return cue.title.isEmpty ? "Pad \(pad % max(show.slots, 1) + 1)" : cue.title
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

    private func applyRemote() {
        do {
            try session.setRemote(port: remoteEnabled ? remotePort : 0, pin: remotePin)
        } catch {
            alert = AlertMessage(title: "Remote", message: error.localizedDescription)
        }
        refreshStatus()
    }

    /// A new PIN; phones using the old one are asked for it again.
    func regenerateRemotePin() {
        remotePin = Self.newPin()
        UserDefaults.standard.set(remotePin, forKey: "remotePin")
        applyRemote()
    }

    private static func newPin() -> String {
        // Six digits, leading zeros kept.
        String(String(Int.random(in: 1_000_000...1_999_999)).dropFirst())
    }

    private static func savedOrNewPin() -> String {
        if let pin = UserDefaults.standard.string(forKey: "remotePin"), pin.count == 6 {
            return pin
        }
        let pin = newPin()
        UserDefaults.standard.set(pin, forKey: "remotePin")
        return pin
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
        refreshShow(undo: "Clear MIDI")
    }

    var learningTarget: String? { status?.learning }

    func runPreflight() {
        preflight = session.preflight()
    }


    func applyOutputs(_ outputs: [OutputDoc]) {
        report { try session.setOutputs(outputs) }
        refreshShow(undo: "Audio Outputs")
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
        var cue = show.cue(at: slot) ?? newPad(slot: slot)
        if cue.playlist == nil {
            cue.title = "Playlist"
        }
        cue.clips = []
        cue.markers = []
        cue.playlist = PlaylistDoc(items: urls.map(\.path))
        if cue.hotkey == nil {
            cue.hotkey = freeHotkey(preferring: slot)
        }
        updateCue(cue, undo: "Make Playlist")
    }

    // MARK: - Play log

    /// Daily log files (one JSON line per play) live here.
    static var logDirectory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first ?? URL(fileURLWithPath: NSTemporaryDirectory())
        return base.appendingPathComponent("Sportsball Jukebox/Logs", isDirectory: true)
    }

    var game: String { status?.game ?? "" }

    func setGame(_ name: String) {
        session.setGame(name)
        refreshStatus()
    }

    func playLog() -> PlayLog {
        session.playLog()
    }

    func clearLog() {
        session.clearLog()
    }

    func openLogFolder() {
        try? FileManager.default.createDirectory(at: Self.logDirectory, withIntermediateDirectories: true)
        NSWorkspace.shared.open(Self.logDirectory)
    }

    /// Writes the log as CSV (local date and time) for licensing or
    /// sponsor reports.
    func exportLog(_ log: PlayLog) {
        let panel = NSSavePanel()
        panel.title = "Export Play Log"
        panel.allowedContentTypes = [.commaSeparatedText]
        let day = ISO8601DateFormatter.string(from: Date(), timeZone: .current, formatOptions: [.withFullDate])
        panel.nameFieldStringValue = "Play log \(game.isEmpty ? day : game).csv"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let date = DateFormatter()
        date.dateFormat = "yyyy-MM-dd"
        let time = DateFormatter()
        time.dateFormat = "HH:mm:ss"
        func field(_ s: String) -> String {
            "\"" + s.replacingOccurrences(of: "\"", with: "\"\"") + "\""
        }
        var lines = ["Date,Time,Game,Show,Bank,Pad,Title,File,Sponsor,Triggered by,Output,Seconds"]
        for e in log.entries {
            let row: [String] = [
                date.string(from: e.startDate),
                time.string(from: e.startDate),
                field(e.game),
                field(e.show),
                String(e.bank),
                String(e.pad),
                field(e.title),
                field(e.fileName),
                field(e.sponsor ?? ""),
                e.source,
                field(e.output),
                String(format: "%.1f", e.seconds),
            ]
            lines.append(row.joined(separator: ","))
        }
        do {
            try lines.joined(separator: "\n").write(to: url, atomically: true, encoding: .utf8)
        } catch {
            alert = AlertMessage(title: "Could not export", message: error.localizedDescription)
        }
    }

    // MARK: - Preview

    var canPreview: Bool { show.previewOutput != nil }

    /// Plays a pad on the preview output (e.g. headphones); again stops it.
    func preview(_ slot: Int) {
        guard canPreview else {
            alert = AlertMessage(
                title: "No preview output",
                message: "Choose a preview output (for example headphones) in Audio Outputs."
            )
            return
        }
        report { try session.preview(slot: slot) }
        refreshStatus()
    }

    func stopPreview() {
        session.stopPreview()
        refreshStatus()
    }

    func setPreviewOutput(_ output: Int?) {
        report { try session.setPreviewOutput(output) }
        refreshShow(undo: "Preview Output")
    }

    /// Pads playing now, soonest to end first (countdown-less ones last).
    var nowPlaying: [NowPlayingItem] {
        show.cues
            .map { NowPlayingItem(cue: $0, state: state(of: $0.slot)) }
            .filter { $0.state.playing }
            .sorted { ($0.state.remaining ?? .infinity) < ($1.state.remaining ?? .infinity) }
    }

    /// Playlist pad: crossfade to the next song.
    func skip(_ slot: Int) {
        report { try session.skip(slot: slot) }
    }

    /// Puts a file on a pad. An existing cue keeps its settings (key,
    /// output, volume) but gets the new sound.
    func assignSound(url: URL, to slot: Int) {
        let title = url.deletingPathExtension().lastPathComponent
        let isNew = show.cue(at: slot) == nil
        var cue = show.cue(at: slot) ?? newPad(slot: slot)
        cue.title = title
        cue.clips = [ClipDoc(file: url.path)]
        cue.markers = []
        cue.playlist = nil
        if cue.hotkey == nil {
            cue.hotkey = freeHotkey(preferring: slot)
        }
        let trim = isNew ? (show.padDefaults?.trimSilence ?? true) : true
        updateCue(cue, trimLeadingSilence: trim, undo: "Assign Sound")
    }

    /// An empty pad with the show's defaults for new pads.
    private func newPad(slot: Int) -> CueDoc {
        var cue = CueDoc(slot: slot)
        show.padDefaults?.apply(to: &cue, outputs: show.outputs.count)
        return cue
    }

    // MARK: - Pad style and pictures

    func copyStyle(from slot: Int) {
        copiedStyle = show.cue(at: slot).map(PadStyle.init(cue:))
    }

    func pasteStyle(to slot: Int) {
        guard let style = copiedStyle, var cue = show.cue(at: slot) else { return }
        style.apply(to: &cue, outputs: show.outputs.count)
        updateCue(cue, undo: "Paste Style")
    }

    /// New pads get this pad's colour, fades, loop, group, output and
    /// ducking.
    func useStyleAsDefault(from slot: Int) {
        guard let cue = show.cue(at: slot) else { return }
        var defaults = show.padDefaults ?? PadDefaults()
        defaults.color = cue.color
        defaults.fadeIn = cue.fadeIn
        defaults.fadeOut = cue.fadeOut
        defaults.loop = cue.loop
        defaults.group = cue.group
        defaults.output = cue.output
        defaults.duck = cue.duck
        defaults.levelMatch = cue.levelMatch
        setPadDefaults(defaults)
    }

    func setPadDefaults(_ defaults: PadDefaults?) {
        report { try session.setPadDefaults(defaults) }
        refreshShow(undo: "New Pad Defaults")
    }

    func choosePicture(for slot: Int) {
        let panel = NSOpenPanel()
        panel.title = "Choose a picture for the pad"
        panel.allowedContentTypes = [.image]
        if panel.runModal() == .OK, let url = panel.url {
            setImage(url, for: slot)
        }
    }

    /// Sets (or with nil removes) a pad's picture. Once the show is saved,
    /// the picture is copied into an Images folder beside it, so it moves
    /// with the show.
    func setImage(_ url: URL?, for slot: Int) {
        guard var cue = show.cue(at: slot) else { return }
        cue.image = url.map(storedImagePath(for:))
        updateCue(cue, undo: url == nil ? "Remove Picture" : "Picture")
    }

    /// Where a chosen picture is kept: copied beside a saved show.
    func storedImagePath(for url: URL) -> String {
        copyIntoShowFolder(url, subfolder: "Images").path
    }

    private func copyIntoShowFolder(_ url: URL, subfolder: String) -> URL {
        guard let path = status?.path, !isAutosaveFile(path) else { return url }
        let folder = URL(fileURLWithPath: path).deletingLastPathComponent()
            .appendingPathComponent(subfolder, isDirectory: true)
        if url.deletingLastPathComponent().standardizedFileURL == folder.standardizedFileURL {
            return url
        }
        let destination = folder.appendingPathComponent(url.lastPathComponent)
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            if !FileManager.default.fileExists(atPath: destination.path) {
                try FileManager.default.copyItem(at: url, to: destination)
            }
            return destination
        } catch {
            return url
        }
    }

    /// A pad's picture, loaded once (nil if none or not found).
    func padImage(for cue: CueDoc) -> NSImage? {
        guard let stored = cue.image else { return nil }
        if let image = imageCache[stored] { return image }
        if missingImages.contains(stored) { return nil }
        guard let path = session.resolveFile(stored), let image = NSImage(contentsOfFile: path) else {
            missingImages.insert(stored)
            return nil
        }
        imageCache[stored] = image
        return image
    }

    // MARK: - Library

    static let audioExtensions: Set<String> = [
        "wav", "wave", "aif", "aiff", "aifc", "caf", "flac", "mp3", "m4a", "aac", "mp4", "alac", "ogg", "oga",
    ]

    /// Adds files, and the audio files inside folders, to the library.
    func addToLibrary(urls: [URL]) {
        var files: [String] = []
        for url in urls {
            var isFolder: ObjCBool = false
            guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isFolder) else { continue }
            if isFolder.boolValue {
                let walker = FileManager.default.enumerator(
                    at: url, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles]
                )
                while let item = walker?.nextObject() as? URL {
                    if Self.audioExtensions.contains(item.pathExtension.lowercased()) {
                        files.append(item.path)
                    }
                }
            } else if Self.audioExtensions.contains(url.pathExtension.lowercased()) {
                files.append(url.path)
            }
        }
        guard !files.isEmpty else { return }
        report { try session.addToLibrary(files.sorted()) }
        showingLibrary = true
        refreshShow(undo: "Add to Library")
    }

    func importToLibrary() {
        let panel = NSOpenPanel()
        panel.title = "Add to Library"
        panel.message = "Choose audio files or folders."
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        panel.allowedContentTypes = [.audio, .folder]
        if panel.runModal() == .OK {
            addToLibrary(urls: panel.urls)
        }
    }

    func removeFromLibrary(_ file: String) {
        report { try session.removeFromLibrary(file) }
        refreshShow(undo: "Remove from Library")
    }

    /// The file on disk (it may have moved with the show).
    func libraryURL(_ file: String) -> URL {
        URL(fileURLWithPath: session.resolveFile(file) ?? file)
    }

    @Published private(set) var loadingPreview: String?

    /// Plays a library file in the headphones (decoding it first, in the
    /// background); again stops it.
    func previewFile(_ file: String) {
        guard canPreview else {
            alert = AlertMessage(
                title: "No preview output",
                message: "Choose a preview output (for example headphones) in Audio Outputs."
            )
            return
        }
        loadingPreview = file
        let session = self.session
        Task.detached(priority: .userInitiated) {
            var failure: String?
            do {
                try session.previewFile(file)
            } catch {
                failure = error.localizedDescription
            }
            let message = failure
            await MainActor.run {
                self.loadingPreview = nil
                if let message {
                    self.alert = AlertMessage(title: "Could not preview", message: message)
                }
                self.refreshShow()
            }
        }
    }

    /// Below this, leading silence is left alone.
    static let minimumTrimSeconds = 0.01

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
    /// `trimLeadingSilence` (for a newly dropped file) starts the first
    /// clip where the sound starts, so the pad fires instantly.
    func updateCue(_ cue: CueDoc, trimLeadingSilence: Bool = false, undo label: String = "Edit Pad") {
        busySlots.insert(cue.slot)
        let session = self.session
        let minimumTrim = Self.minimumTrimSeconds
        Task.detached(priority: .userInitiated) {
            var cue = cue
            var failure: String?
            var info: (String, SoundInfo)?
            do {
                if trimLeadingSilence, let file = cue.clips.first?.file {
                    _ = try session.loadSound(file: file)
                    if let measured = session.soundInfo(file: file) {
                        info = (file, measured)
                        if measured.leadSilence >= minimumTrim,
                           measured.leadSilence < measured.duration {
                            cue.clips[0].offset = measured.leadSilence
                        }
                    }
                }
                try session.setCue(cue)
            } catch {
                failure = error.localizedDescription
            }
            let message = failure
            let measured = info
            let slot = cue.slot
            await MainActor.run {
                if let measured {
                    self.soundInfos[measured.0] = measured.1
                }
                self.busySlots.remove(slot)
                if let message {
                    self.alert = AlertMessage(title: "Could not update pad", message: message)
                }
                self.refreshShow(undo: label)
            }
        }
    }

    func clear(_ slot: Int) {
        report { try session.removeCue(slot: slot) }
        refreshShow(undo: "Clear Pad")
    }

    func move(from: Int, to: Int) {
        guard from != to else { return }
        report { try session.moveCue(from: from, to: to) }
        refreshShow(undo: "Move Pad")
    }

    func setLoop(_ slot: Int, _ looping: Bool) {
        guard var cue = show.cue(at: slot) else { return }
        cue.loop = looping
        updateCue(cue, undo: "Loop")
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
            let info = session.soundInfo(file: file)
            await MainActor.run {
                if let duration {
                    self.durations[file] = duration
                }
                if let peaks {
                    self.waveforms[file] = peaks
                }
                if let info {
                    self.soundInfos[file] = info
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

/// What Copy Style / Paste Style carries between pads: look and
/// behaviour, not the sound, title or key.
struct PadStyle: Equatable {
    var color: String?
    var image: String?
    var fadeIn: Double
    var fadeOut: Double?
    var loop: Bool
    var group: String?
    var output: Int
    var duck: Float?
    var retrigger: Retrigger
    var levelMatch: Bool

    init(cue: CueDoc) {
        color = cue.color
        image = cue.image
        fadeIn = cue.fadeIn
        fadeOut = cue.fadeOut
        loop = cue.loop
        group = cue.group
        output = cue.output
        duck = cue.duck
        retrigger = cue.retrigger
        levelMatch = cue.levelMatch
    }

    func apply(to cue: inout CueDoc, outputs: Int) {
        cue.color = color
        cue.image = image
        cue.fadeIn = fadeIn
        cue.fadeOut = fadeOut
        cue.loop = loop
        cue.group = group
        cue.output = output < outputs ? output : 0
        cue.duck = duck
        cue.retrigger = retrigger
        cue.levelMatch = levelMatch
    }
}
