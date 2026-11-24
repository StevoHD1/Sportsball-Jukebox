// Swift wrapper over the Rust engine's C API (crates/sbj-ffi).
//
// The Rust session is internally locked and its audio runs on Core Audio's
// own real-time threads, so these wrappers are safe to call from any
// thread. Calls marked "slow" decode audio and belong off the main thread.

import Foundation
import SbjEngineC

struct EngineFailure: LocalizedError, Sendable {
    let code: UInt32
    let message: String

    var errorDescription: String? { message }
}

enum Engine {
    static var version: String { String(cString: sbj_version()) }

    /// The calling thread's most recent error message from the engine.
    static func lastError() -> String {
        var buffer = [CChar](repeating: 0, count: 1024)
        _ = sbj_last_error_message(&buffer, buffer.count)
        return buffer.withUnsafeBufferPointer { String(cString: $0.baseAddress!) }
    }

    static func check(_ status: SbjStatus) throws {
        if status != SBJ_STATUS_OK {
            throw EngineFailure(
                code: UInt32(truncatingIfNeeded: status.rawValue),
                message: lastError()
            )
        }
    }

    /// Whether the compiled-in header matches the linked library's structs.
    static var abiMatches: Bool {
        sbj_abi_check(MemoryLayout<SbjDeviceInfo>.stride, MemoryLayout<SbjCueState>.stride)
    }

    /// Takes ownership of a string returned by the engine.
    static func take(_ pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        defer { sbj_string_free(pointer) }
        return String(cString: pointer)
    }
}

/// Reads a NUL-terminated string out of an imported fixed-size C char array.
private func string<T>(fromCArray array: T) -> String {
    withUnsafeBytes(of: array) { raw in
        String(decoding: raw.prefix(while: { $0 != 0 }), as: UTF8.self)
    }
}

struct AudioDevice: Identifiable, Hashable, Sendable {
    let id: String
    let name: String
    let channels: Int
    let sampleRate: Int
    let isDefault: Bool
}

struct CueStatus: Equatable, Sendable {
    var assigned = false
    var playing = false
    var looping = false
    var ready = false
    var position: Double = 0
    var duration: Double = 0

    var progress: Double {
        duration > 0 ? min(max(position / duration, 0), 1) : 0
    }
}

/// One engine session: a show playing through one or more devices.
final class EngineSession: @unchecked Sendable {
    private let handle: OpaquePointer

    init() throws {
        guard let handle = sbj_session_create() else {
            throw EngineFailure(code: 0, message: Engine.lastError())
        }
        self.handle = handle
    }

    deinit { sbj_session_destroy(handle) }

    // MARK: Devices and housekeeping

    func openOutputs() {
        _ = sbj_session_open_outputs(handle)
    }

    func poll() {
        _ = sbj_session_poll(handle)
    }

    func devices() -> [AudioDevice] {
        let count = sbj_session_device_count(handle)
        return (0..<count).compactMap { index in
            var info = SbjDeviceInfo()
            guard sbj_session_device_info(handle, index, &info) == SBJ_STATUS_OK else {
                return nil
            }
            return AudioDevice(
                id: string(fromCArray: info.id),
                name: string(fromCArray: info.name),
                channels: Int(info.output_channels),
                sampleRate: Int(info.sample_rate),
                isDefault: info.is_default
            )
        }
    }

    // MARK: Show

    func show() -> ShowDoc? {
        guard let json = Engine.take(sbj_session_show_json(handle)) else { return nil }
        return try? ShowDoc.decode(json)
    }

    func status() -> SessionStatus? {
        guard let json = Engine.take(sbj_session_status_json(handle)) else { return nil }
        return try? SessionStatus.decode(json)
    }

    func newShow() throws {
        try Engine.check(sbj_session_new_show(handle))
    }

    /// Slow: decodes all of the show's audio.
    func openShow(at url: URL) throws {
        try Engine.check(sbj_session_open_show(handle, url.path))
    }

    /// Saves to `url`, or to the show's current file if nil.
    func saveShow(to url: URL?) throws {
        if let url {
            try Engine.check(sbj_session_save_show(handle, url.path))
        } else {
            try Engine.check(sbj_session_save_show(handle, nil))
        }
    }

    func setShowSettings(name: String, slots: Int, columns: Int) throws {
        try Engine.check(sbj_session_set_show_settings(
            handle, name, UInt32(clamping: slots), UInt32(clamping: columns)
        ))
    }

    func setOutputs(_ outputs: [OutputDoc]) throws {
        try Engine.check(sbj_session_set_outputs_json(handle, try ShowDoc.encode(outputs)))
    }

    // MARK: Cues

    /// Slow if the cue uses audio that is not loaded yet.
    func setCue(_ cue: CueDoc) throws {
        try Engine.check(sbj_session_set_cue_json(handle, try ShowDoc.encode(cue)))
    }

    func removeCue(slot: Int) throws {
        try Engine.check(sbj_session_remove_cue(handle, UInt32(slot)))
    }

    func moveCue(from: Int, to: Int) throws {
        try Engine.check(sbj_session_move_cue(handle, UInt32(from), UInt32(to)))
    }

    /// Slow: decodes a file so it can be displayed. Returns its duration.
    func loadSound(file: String) throws -> Double {
        var duration = 0.0
        try Engine.check(sbj_session_load_sound(handle, file, &duration))
        return duration
    }

    /// (min, max) pairs across the whole file.
    func waveform(file: String, buckets: Int) -> [Float]? {
        var peaks = [Float](repeating: 0, count: buckets * 2)
        let status = sbj_session_waveform(handle, file, UInt32(clamping: buckets), &peaks)
        return status == SBJ_STATUS_OK ? peaks : nil
    }

    // MARK: Playback

    func fire(slot: Int) throws {
        try Engine.check(sbj_session_fire_cue(handle, UInt32(slot)))
    }

    func play(slot: Int, from seconds: Double) throws {
        try Engine.check(sbj_session_play_cue_from(handle, UInt32(slot), seconds))
    }

    func stop(slot: Int, fadeSeconds: Double) throws {
        try Engine.check(sbj_session_stop_cue(handle, UInt32(slot), fadeSeconds))
    }

    func stopAll(fadeSeconds: Double) throws {
        try Engine.check(sbj_session_stop_all(handle, fadeSeconds))
    }

    func setMasterGain(_ gain: Float) throws {
        try Engine.check(sbj_session_set_master_gain(handle, gain))
    }

    func cueStates(count: Int) -> [CueStatus] {
        guard count > 0 else { return [] }
        var raw = [SbjCueState](repeating: SbjCueState(), count: count)
        guard sbj_session_cue_states(handle, 0, UInt32(count), &raw) == SBJ_STATUS_OK else {
            return Array(repeating: CueStatus(), count: count)
        }
        return raw.map {
            CueStatus(
                assigned: $0.assigned,
                playing: $0.playing,
                looping: $0.looping,
                ready: $0.ready,
                position: $0.position_seconds,
                duration: $0.duration_seconds
            )
        }
    }
}
