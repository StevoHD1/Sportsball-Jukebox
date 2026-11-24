import AppKit
import Foundation
import UniformTypeIdentifiers

struct Cue: Identifiable, Equatable {
    let id: Int
    let hotkey: Character?
    var title: String?
    var isLoading = false
    var playing = false
    var looping = false
    var position: Double = 0
    var duration: Double = 0

    var isAssigned: Bool { title != nil }

    var progress: Double {
        duration > 0 ? min(max(position / duration, 0), 1) : 0
    }
}

struct AlertMessage: Identifiable {
    let id = UUID()
    let title: String
    let message: String
}

/// UI state, kept in sync with the Rust engine session.
@MainActor
final class AppModel: ObservableObject {
    static let columns = 6
    static let hotkeys = Array("1234567890qwertyuiopasdf")
    static let cueCount = hotkeys.count

    /// Fade used by the Stop All button and per-cue "Fade Out".
    static let stopFadeSeconds = 0.5

    @Published private(set) var cues: [Cue]
    @Published private(set) var devices: [AudioDevice] = []
    @Published private(set) var output: OutputInfo?
    @Published private(set) var selectedDeviceID: String?
    @Published private(set) var isOpeningOutput = false
    @Published var alert: AlertMessage?
    @Published var masterVolume: Double = 1.0 {
        didSet { report { try session.setMasterGain(Self.gain(forSlider: masterVolume)) } }
    }

    let session: EngineSession
    // Runs for the life of the app (the model is never deallocated).
    private var pollTask: Task<Void, Never>?
    private var activity: NSObjectProtocol?

    init() {
        guard Engine.abiMatches else {
            fatalError("Engine header and library are out of sync; rebuild the Rust engine.")
        }
        do {
            session = try EngineSession(cueCount: Self.cueCount)
        } catch {
            fatalError("Could not create engine session: \(error.localizedDescription)")
        }
        cues = (0..<Self.cueCount).map { Cue(id: $0, hotkey: Self.hotkeys[$0]) }

        // Keep macOS from throttling us (App Nap) during a live show.
        activity = ProcessInfo.processInfo.beginActivity(
            options: [.userInitiated, .latencyCritical],
            reason: "Live audio playback"
        )

        refreshDevices()
        selectDevice(nil)

        pollTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                self?.tick()
                try? await Task.sleep(nanoseconds: 33_000_000)  // ~30 Hz
            }
        }
    }

    /// Slider position (0...1) to linear gain, with a perceptual curve.
    static func gain(forSlider value: Double) -> Float {
        Float(value * value)
    }

    // MARK: - Devices

    func refreshDevices() {
        devices = session.devices()
    }

    /// Opens a device (nil = system default) off the main thread.
    func selectDevice(_ id: String?) {
        selectedDeviceID = id
        isOpeningOutput = true
        let session = self.session
        Task.detached(priority: .userInitiated) {
            var failure: String?
            do {
                try session.openOutput(deviceID: id)
            } catch {
                failure = error.localizedDescription
            }
            let message = failure
            await MainActor.run {
                self.isOpeningOutput = false
                self.output = session.outputInfo()
                if let message {
                    self.alert = AlertMessage(title: "Could not open audio output", message: message)
                }
            }
        }
    }

    // MARK: - Cues

    /// Clicking a cue: fire it if it has a sound, otherwise choose one.
    func trigger(_ cue: Int) {
        if cues[cue].isAssigned {
            report { try session.fire(cue: cue) }
        } else if !cues[cue].isLoading {
            chooseSound(for: cue)
        }
    }

    func chooseSound(for cue: Int) {
        let panel = NSOpenPanel()
        panel.title = "Choose a sound for cue \(cue + 1)"
        panel.allowedContentTypes = [.audio]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        if panel.runModal() == .OK, let url = panel.url {
            loadSound(url: url, into: cue)
        }
    }

    /// Decodes a file in the background, then assigns it.
    func loadSound(url: URL, into cue: Int) {
        guard cues.indices.contains(cue) else { return }
        cues[cue].isLoading = true
        let targetRate = output?.sampleRate ?? 0
        let session = self.session
        Task.detached(priority: .userInitiated) {
            let loaded = LoadedSound.load(url: url, targetSampleRate: targetRate)
            let assigned = loaded.flatMap { sound in
                Result { try session.setSound(sound, cue: cue) }
                    .mapError { EngineFailure(code: 0, message: $0.localizedDescription) }
                    .map { sound }
            }
            await MainActor.run {
                self.cues[cue].isLoading = false
                switch assigned {
                case .success(let sound):
                    self.cues[cue].title = url.deletingPathExtension().lastPathComponent
                    self.cues[cue].duration = sound.duration
                    self.cues[cue].position = 0
                case .failure(let error):
                    self.alert = AlertMessage(
                        title: "Could not load \(url.lastPathComponent)",
                        message: error.localizedDescription
                    )
                }
            }
        }
    }

    func stop(_ cue: Int, fade: Bool) {
        report { try session.stop(cue: cue, fadeSeconds: fade ? Self.stopFadeSeconds : 0) }
    }

    func stopAll() {
        report { try session.stopAll(fadeSeconds: Self.stopFadeSeconds) }
    }

    func setLoop(_ cue: Int, _ looping: Bool) {
        cues[cue].looping = looping
        report { try session.setLoop(cue: cue, looping: looping) }
    }

    func clear(_ cue: Int) {
        report { try session.clear(cue: cue) }
        cues[cue].title = nil
        cues[cue].duration = 0
        cues[cue].position = 0
    }

    // MARK: - Housekeeping

    private func tick() {
        session.poll()

        var updated = cues
        for index in updated.indices {
            guard let status = session.status(cue: index) else { continue }
            updated[index].playing = status.playing
            updated[index].looping = status.looping
            updated[index].position = status.position
        }
        if updated != cues {
            cues = updated
        }

        let info = session.outputInfo()
        if info != output {
            output = info
        }
    }

    /// Runs an engine call, surfacing any failure as an alert.
    private func report(_ action: () throws -> Void) {
        do {
            try action()
        } catch {
            alert = AlertMessage(title: "Engine error", message: error.localizedDescription)
        }
    }
}
