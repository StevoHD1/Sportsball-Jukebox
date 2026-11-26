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
    /// Vamping: pressing the pad lets it continue.
    var holding = false
    /// Playlist pads: index of the current song in the playlist.
    var item: Int?
    /// Playlist pads: the song that plays next.
    var nextItem: Int?

    /// Seconds left, if counting down makes sense (not looping or vamping).
    var remaining: Double? {
        guard playing, !holding, !looping, duration > 0 else { return nil }
        return max(duration - position, 0)
    }
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

    func setShowSettings(
        name: String, slots: Int, columns: Int, banks: [String], stopAllFade: Double,
        levelMatch: Double?, playedColor: String?, markPlayed: Bool
    ) throws {
        struct Settings: Encodable {
            let name: String
            let slots: Int
            let columns: Int
            let banks: [String]
            let stopAllFade: Double
            /// Always written: null turns level matching off.
            let levelMatch: Double?
            /// Always written: null turns the played colour off.
            let playedColor: String?
            let markPlayed: Bool

            enum CodingKeys: String, CodingKey {
                case name, slots, columns, banks
                case stopAllFade = "stop_all_fade"
                case levelMatch = "level_match"
                case playedColor = "played_color"
                case markPlayed = "mark_played"
            }

            func encode(to encoder: Encoder) throws {
                var c = encoder.container(keyedBy: CodingKeys.self)
                try c.encode(name, forKey: .name)
                try c.encode(slots, forKey: .slots)
                try c.encode(columns, forKey: .columns)
                try c.encode(banks, forKey: .banks)
                try c.encode(stopAllFade, forKey: .stopAllFade)
                try c.encode(levelMatch, forKey: .levelMatch)
                try c.encode(playedColor, forKey: .playedColor)
                try c.encode(markPlayed, forKey: .markPlayed)
            }
        }
        let json = try ShowDoc.encode(Settings(
            name: name, slots: slots, columns: columns, banks: banks, stopAllFade: stopAllFade,
            levelMatch: levelMatch, playedColor: playedColor, markPlayed: markPlayed
        ))
        try Engine.check(sbj_session_set_show_settings_json(handle, json))
    }

    // MARK: Undo and Save With Audio

    /// Marks the show's changes since the last mark as one undo step.
    /// Returns whether anything changed.
    func recordUndo(_ label: String) -> Bool {
        sbj_session_record_undo(handle, label)
    }

    func undo() throws {
        try Engine.check(sbj_session_undo(handle))
    }

    func redo() throws {
        try Engine.check(sbj_session_redo(handle))
    }

    /// Slow: copies every file the show uses beside `url` and saves there.
    func saveWithAudio(to url: URL) throws {
        try Engine.check(sbj_session_save_with_audio(handle, url.path))
    }

    // MARK: Played marks

    func setPlayed(slot: Int, _ played: Bool) {
        _ = sbj_session_set_played(handle, UInt32(clamping: slot), played)
    }

    /// nil = every bank.
    func resetPlayed(bank: Int?) {
        _ = sbj_session_reset_played(handle, Int32(clamping: bank ?? -1))
    }

    // MARK: Diagnostics

    func events() -> [EventEntry] {
        guard let json = Engine.take(sbj_session_events_json(handle)) else { return [] }
        return EventEntry.decodeList(json)
    }

    func resetGlitches() {
        _ = sbj_session_reset_glitches(handle)
    }

    // MARK: Colour presets

    func setShowColors(_ colors: [ShowColor], recolor: Bool) throws {
        try Engine.check(sbj_session_set_show_colors_json(handle, try ShowDoc.encode(colors), recolor))
    }

    // MARK: Live transport

    func seekCue(slot: Int, seconds: Double) throws {
        try Engine.check(sbj_session_seek_cue(handle, UInt32(clamping: slot), seconds))
    }

    func seekPreview(seconds: Double) throws {
        try Engine.check(sbj_session_seek_preview(handle, seconds))
    }

    func stopLast(fade: Bool) throws {
        try Engine.check(sbj_session_stop_last(handle, fade))
    }

    func setBankMultiplay(_ bank: Int, _ multiplay: Bool) throws {
        try Engine.check(sbj_session_set_bank_multiplay(handle, UInt32(clamping: bank), multiplay))
    }

    // MARK: Library

    func library() -> [LibraryEntry] {
        guard let json = Engine.take(sbj_session_library_json(handle)) else { return [] }
        return LibraryEntry.decodeList(json)
    }

    func addToLibrary(_ files: [String]) throws {
        try Engine.check(sbj_session_add_to_library_json(handle, try ShowDoc.encode(files)))
    }

    func removeFromLibrary(_ file: String) throws {
        try Engine.check(sbj_session_remove_from_library(handle, file))
    }

    /// Slow the first time (decodes). Previewing the same file again stops it.
    func previewFile(_ file: String) throws {
        try Engine.check(sbj_session_preview_file(handle, file))
    }

    /// Where a file the show refers to is on disk (also looks in the
    /// show's Audio and Images folders), or nil.
    func resolveFile(_ file: String) -> String? {
        Engine.take(sbj_session_resolve_file(handle, file))
    }

    /// nil restores the built-in defaults.
    func setPadDefaults(_ defaults: PadDefaults?) throws {
        if let defaults {
            try Engine.check(sbj_session_set_pad_defaults_json(handle, try ShowDoc.encode(defaults)))
        } else {
            try Engine.check(sbj_session_set_pad_defaults_json(handle, nil))
        }
    }

    // MARK: Rundown

    func setRundown(_ items: [RundownItem]) throws {
        try Engine.check(sbj_session_set_rundown_json(handle, try ShowDoc.encode(items)))
    }

    func rundownGo() throws {
        try Engine.check(sbj_session_rundown_go(handle))
    }

    /// nil = finished.
    func setStandby(_ index: Int?) {
        _ = sbj_session_set_standby(handle, Int32(clamping: index ?? -1))
    }

    func stepStandby(forward: Bool) {
        _ = sbj_session_step_standby(handle, forward)
    }

    func setBank(_ bank: Int) throws {
        try Engine.check(sbj_session_set_bank(handle, UInt32(clamping: bank)))
    }

    // MARK: External control

    func enableMIDI(_ enabled: Bool) throws {
        try Engine.check(sbj_session_enable_midi(handle, enabled))
    }

    /// 0 turns OSC off.
    func setOSCPort(_ port: Int) throws {
        try Engine.check(sbj_session_set_osc_port(handle, UInt16(clamping: port)))
    }

    /// Serves the phone/tablet remote page on a TCP port; 0 stops it.
    /// `pin` must be 4–12 digits.
    func setRemote(port: Int, pin: String) throws {
        try Engine.check(sbj_session_set_remote(handle, UInt16(clamping: port), pin))
    }

    /// Learns the next MIDI message for a target ("cue:<slot>",
    /// "stop_all", "next_bank", "prev_bank"); nil cancels.
    func learnMIDI(target: String?) throws {
        if let target {
            try Engine.check(sbj_session_learn_midi(handle, target))
        } else {
            try Engine.check(sbj_session_learn_midi(handle, nil))
        }
    }

    func clearMIDI(target: String) throws {
        try Engine.check(sbj_session_clear_midi(handle, target))
    }

    func preflight() -> PreflightReport? {
        guard let json = Engine.take(sbj_session_preflight_json(handle)) else { return nil }
        return try? PreflightReport.decode(json)
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

    /// Loudness, peak and silence of a loaded file (after `loadSound`).
    func soundInfo(file: String) -> SoundInfo? {
        guard let json = Engine.take(sbj_session_sound_info_json(handle, file)) else {
            return nil
        }
        return SoundInfo.decode(json)
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

    // MARK: Play log

    func setLogDirectory(_ url: URL?) {
        if let url {
            _ = sbj_session_set_log_dir(handle, url.path)
        } else {
            _ = sbj_session_set_log_dir(handle, nil)
        }
    }

    func setGame(_ name: String) {
        _ = sbj_session_set_game(handle, name)
    }

    func playLog() -> PlayLog {
        guard let json = Engine.take(sbj_session_log_json(handle)) else { return .empty }
        return (try? PlayLog.decode(json)) ?? .empty
    }

    func clearLog() {
        _ = sbj_session_clear_log(handle)
    }

    /// Plays a pad on the preview output; again stops it.
    func preview(slot: Int) throws {
        try Engine.check(sbj_session_preview_cue(handle, UInt32(slot)))
    }

    func stopPreview() {
        _ = sbj_session_stop_preview(handle)
    }

    func setPreviewOutput(_ output: Int?) throws {
        try Engine.check(sbj_session_set_preview_output(handle, Int32(output ?? -1)))
    }

    /// Playlist pad: crossfade to the next song.
    func skip(slot: Int) throws {
        try Engine.check(sbj_session_skip_cue(handle, UInt32(slot)))
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
                holding: $0.holding,
                item: $0.item == UInt32.max ? nil : Int($0.item),
                nextItem: $0.next_item == UInt32.max ? nil : Int($0.next_item),
                position: $0.position_seconds,
                duration: $0.duration_seconds
            )
        }
    }
}
