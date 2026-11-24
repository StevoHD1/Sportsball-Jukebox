// Swift wrapper over the Rust engine's C API (crates/sbj-ffi).
//
// The Rust session is internally locked and its audio runs on Core Audio's
// own real-time thread, so these wrappers are safe to call from any thread.

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
            throw EngineFailure(code: UInt32(truncatingIfNeeded: status.rawValue), message: lastError())
        }
    }

    /// Whether the compiled-in header matches the linked library's structs.
    static var abiMatches: Bool {
        sbj_abi_check(
            MemoryLayout<SbjDeviceInfo>.stride,
            MemoryLayout<SbjOutputInfo>.stride,
            MemoryLayout<SbjCueState>.stride
        )
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

enum OutputHealth: Sendable {
    case running, deviceLost, failed
}

struct OutputInfo: Equatable, Sendable {
    let deviceID: String
    let sampleRate: Int
    let channels: Int
    let health: OutputHealth
    let xruns: UInt64
}

struct CueStatus: Equatable, Sendable {
    let assigned: Bool
    let playing: Bool
    let looping: Bool
    let gain: Float
    let position: Double
    let duration: Double
}

/// A fully decoded sound. Decoding is slow; create these off the main thread.
final class LoadedSound: @unchecked Sendable {
    fileprivate let handle: OpaquePointer
    let duration: Double

    private init(handle: OpaquePointer) {
        self.handle = handle
        self.duration = sbj_sound_duration_seconds(handle)
    }

    deinit { sbj_sound_free(handle) }

    /// Decodes `url`, converting to `targetSampleRate` (0 = keep native).
    static func load(url: URL, targetSampleRate: Int) -> Result<LoadedSound, EngineFailure> {
        var status = SBJ_STATUS_OK
        let handle = url.path.withCString { path in
            sbj_sound_load(path, UInt32(clamping: targetSampleRate), &status)
        }
        guard let handle else {
            return .failure(EngineFailure(
                code: UInt32(truncatingIfNeeded: status.rawValue),
                message: Engine.lastError()
            ))
        }
        return .success(LoadedSound(handle: handle))
    }
}

/// One engine session: a bank of cues playing through one output device.
final class EngineSession: @unchecked Sendable {
    private let handle: OpaquePointer

    init(cueCount: Int) throws {
        guard let handle = sbj_session_create(UInt32(clamping: cueCount)) else {
            throw EngineFailure(code: 0, message: Engine.lastError())
        }
        self.handle = handle
    }

    deinit { sbj_session_destroy(handle) }

    var backendName: String {
        var buffer = [CChar](repeating: 0, count: 64)
        _ = sbj_session_backend_name(handle, &buffer, buffer.count)
        return buffer.withUnsafeBufferPointer { String(cString: $0.baseAddress!) }
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

    /// Opens a device (nil = system default). May take a moment: every
    /// assigned cue is converted to the device's sample rate.
    func openOutput(deviceID: String?) throws {
        let status: SbjStatus
        if let deviceID {
            status = deviceID.withCString { sbj_session_open_output(handle, $0) }
        } else {
            status = sbj_session_open_output(handle, nil)
        }
        try Engine.check(status)
    }

    func outputInfo() -> OutputInfo? {
        var info = SbjOutputInfo()
        guard sbj_session_output_info(handle, &info) == SBJ_STATUS_OK, info.is_open else {
            return nil
        }
        let health: OutputHealth
        if info.health == SBJ_OUTPUT_DEVICE_LOST {
            health = .deviceLost
        } else if info.health == SBJ_OUTPUT_FAILED {
            health = .failed
        } else {
            health = .running
        }
        return OutputInfo(
            deviceID: string(fromCArray: info.device_id),
            sampleRate: Int(info.sample_rate),
            channels: Int(info.channels),
            health: health,
            xruns: info.xruns
        )
    }

    func setSound(_ sound: LoadedSound, cue: Int) throws {
        try Engine.check(sbj_session_set_cue_sound(handle, UInt32(cue), sound.handle))
    }

    func clear(cue: Int) throws {
        try Engine.check(sbj_session_clear_cue(handle, UInt32(cue)))
    }

    func fire(cue: Int) throws {
        try Engine.check(sbj_session_fire_cue(handle, UInt32(cue)))
    }

    func stop(cue: Int, fadeSeconds: Double) throws {
        try Engine.check(sbj_session_stop_cue(handle, UInt32(cue), fadeSeconds))
    }

    func stopAll(fadeSeconds: Double) throws {
        try Engine.check(sbj_session_stop_all(handle, fadeSeconds))
    }

    func setLoop(cue: Int, looping: Bool) throws {
        try Engine.check(sbj_session_set_cue_loop(handle, UInt32(cue), looping))
    }

    func setGain(cue: Int, gain: Float) throws {
        try Engine.check(sbj_session_set_cue_gain(handle, UInt32(cue), gain))
    }

    func setMasterGain(_ gain: Float) throws {
        try Engine.check(sbj_session_set_master_gain(handle, gain))
    }

    func status(cue: Int) -> CueStatus? {
        var state = SbjCueState()
        guard sbj_session_cue_state(handle, UInt32(cue), &state) == SBJ_STATUS_OK else {
            return nil
        }
        return CueStatus(
            assigned: state.assigned,
            playing: state.playing,
            looping: state.looping,
            gain: state.gain,
            position: state.position_seconds,
            duration: state.duration_seconds
        )
    }
}
