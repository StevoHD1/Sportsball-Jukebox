// Swift mirror of the show file format (crates/sbj-session/src/show.rs).
//
// The Rust side omits fields that have their default value, so every
// decoder here falls back to the same defaults. Keys are snake_case.

import Foundation

extension KeyedDecodingContainer {
    fileprivate func value<T: Decodable>(_ key: Key, or fallback: T) throws -> T {
        try decodeIfPresent(T.self, forKey: key) ?? fallback
    }
}

struct ShowDoc: Codable, Equatable {
    var format = "sportsball-jukebox-show"
    var version = 1
    var name = "Untitled Show"
    /// Pads per bank.
    var slots = 24
    var columns = 6
    var banks: [String] = ["Bank 1"]
    var outputs: [OutputDoc] = [OutputDoc(name: "Main")]
    var cues: [CueDoc] = []
    var controls = ControlsDoc()
    /// Seconds an exclusive group takes to hand over.
    var groupFade: Double = 1.0

    enum CodingKeys: String, CodingKey {
        case format, version, name, slots, columns, banks, outputs, cues, controls
        case groupFade = "group_fade"
    }

    /// Global slot numbers of a bank's pads.
    func padRange(bank: Int) -> Range<Int> {
        let start = bank * slots
        return start..<(start + slots)
    }

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        format = try c.value(.format, or: format)
        version = try c.value(.version, or: version)
        name = try c.value(.name, or: name)
        slots = try c.value(.slots, or: slots)
        columns = try c.value(.columns, or: columns)
        banks = try c.value(.banks, or: banks)
        controls = try c.value(.controls, or: controls)
        groupFade = try c.value(.groupFade, or: groupFade)
        outputs = try c.value(.outputs, or: outputs)
        cues = try c.value(.cues, or: cues)
    }

    func cue(at slot: Int) -> CueDoc? {
        cues.first { $0.slot == slot }
    }

    static func decode(_ json: String) throws -> ShowDoc {
        try JSONDecoder().decode(ShowDoc.self, from: Data(json.utf8))
    }

    static func encode<T: Encodable>(_ value: T) throws -> String {
        String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    }
}

enum MidiKind: String, Codable {
    case note, cc, program
}

struct MidiTrigger: Codable, Equatable, Hashable {
    var kind: MidiKind
    var number: Int
    var channel: Int?

    var label: String {
        let what: String
        switch kind {
        case .note: what = "Note \(number)"
        case .cc: what = "CC \(number)"
        case .program: what = "Program \(number)"
        }
        return channel.map { "\(what) · ch \($0)" } ?? what
    }
}

struct ControlsDoc: Codable, Equatable {
    var stopAll: MidiTrigger?
    var nextBank: MidiTrigger?
    var prevBank: MidiTrigger?

    enum CodingKeys: String, CodingKey {
        case stopAll = "stop_all"
        case nextBank = "next_bank"
        case prevBank = "prev_bank"
    }
}

struct DeviceRef: Codable, Equatable, Hashable {
    var id: String
    var name: String = ""

    enum CodingKeys: String, CodingKey { case id, name }

    init(id: String, name: String) {
        self.id = id
        self.name = name
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.value(.name, or: "")
    }
}

struct OutputDoc: Codable, Equatable {
    var name: String
    var device: DeviceRef?
    var backup: DeviceRef?

    init(name: String, device: DeviceRef? = nil, backup: DeviceRef? = nil) {
        self.name = name
        self.device = device
        self.backup = backup
    }
}

struct CueDoc: Codable, Equatable {
    var slot: Int
    var title = ""
    var hotkey: String?
    var output = 0
    var gain: Float = 1
    var loop = false
    var color: String?
    var clips: [ClipDoc] = []
    var markers: [MarkerDoc] = []
    var midi: MidiTrigger?
    /// Exclusive group: firing this fades out others in the same group.
    var group: String?
    /// dB (negative) to lower other pads on the same output while playing.
    var duck: Float?
    var retrigger: Retrigger = .restart
    var route: RouteDoc?

    enum CodingKeys: String, CodingKey {
        case slot, title, hotkey, output, gain, loop, color, clips, markers, midi
        case group, duck, retrigger, route
    }

    init(slot: Int, title: String, file: String) {
        self.slot = slot
        self.title = title
        self.clips = [ClipDoc(file: file)]
    }

    init(slot: Int) {
        self.slot = slot
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        slot = try c.decode(Int.self, forKey: .slot)
        title = try c.value(.title, or: "")
        hotkey = try c.decodeIfPresent(String.self, forKey: .hotkey)
        output = try c.value(.output, or: 0)
        gain = try c.value(.gain, or: 1)
        loop = try c.value(.loop, or: false)
        color = try c.decodeIfPresent(String.self, forKey: .color)
        clips = try c.value(.clips, or: [])
        markers = try c.value(.markers, or: [])
        midi = try c.decodeIfPresent(MidiTrigger.self, forKey: .midi)
        group = try c.decodeIfPresent(String.self, forKey: .group)
        duck = try c.decodeIfPresent(Float.self, forKey: .duck)
        retrigger = try c.value(.retrigger, or: .restart)
        route = try c.decodeIfPresent(RouteDoc.self, forKey: .route)
    }

    /// Smallest marker id not in use.
    var nextMarkerID: UInt64 {
        (markers.map(\.id).max() ?? 0) + 1
    }
}

enum Retrigger: String, Codable, CaseIterable {
    case restart, stop, fade

    var label: String {
        switch self {
        case .restart: return "Restart"
        case .stop: return "Stop"
        case .fade: return "Fade out"
        }
    }
}

/// Output channels, 1-based.
struct RouteDoc: Codable, Equatable, Hashable {
    var first: Int
    var count: Int

    var label: String {
        count == 1 ? "Channel \(first)" : "Channels \(first)–\(first + count - 1)"
    }
}

enum CurveDoc: String, Codable, CaseIterable {
    case linear
    case equalPower = "equal_power"

    var label: String { self == .linear ? "Linear" : "Equal power" }
}

struct ClipDoc: Codable, Equatable {
    var file: String
    var start: Double = 0
    var offset: Double = 0
    var length: Double?
    var gain: Float = 1
    var fadeIn: Double = 0
    var fadeOut: Double = 0
    var fadeCurve: CurveDoc = .linear

    enum CodingKeys: String, CodingKey {
        case file, start, offset, length, gain
        case fadeIn = "fade_in"
        case fadeOut = "fade_out"
        case fadeCurve = "fade_curve"
    }

    init(file: String) {
        self.file = file
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        file = try c.decode(String.self, forKey: .file)
        start = try c.value(.start, or: 0)
        offset = try c.value(.offset, or: 0)
        length = try c.decodeIfPresent(Double.self, forKey: .length)
        gain = try c.value(.gain, or: 1)
        fadeIn = try c.value(.fadeIn, or: 0)
        fadeOut = try c.value(.fadeOut, or: 0)
        fadeCurve = try c.value(.fadeCurve, or: .linear)
    }

    var fileName: String {
        URL(fileURLWithPath: file).deletingPathExtension().lastPathComponent
    }
}

enum ActionKind: String, Codable, CaseIterable {
    case jump, loop, pause, stop
    case fadeOut = "fade_out"

    var label: String {
        switch self {
        case .jump: return "Jump"
        case .loop: return "Loop"
        case .pause: return "Pause"
        case .stop: return "Stop"
        case .fadeOut: return "Fade Out"
        }
    }

    var needsTarget: Bool { self == .jump || self == .loop }
}

struct TargetDoc: Codable, Equatable {
    var time: Double?
    var marker: UInt64?
}

struct ActionDoc: Codable, Equatable {
    var kind: ActionKind
    var to: TargetDoc?
    var count: UInt32?
    var seconds: Double?
    /// Loop until the pad is pressed again (a vamp).
    var hold: Bool? = nil
}

struct MarkerDoc: Codable, Equatable, Identifiable {
    var id: UInt64
    var name = ""
    var time: Double
    var action: ActionDoc

    enum CodingKeys: String, CodingKey { case id, name, time, action }

    init(id: UInt64, name: String, time: Double, action: ActionDoc) {
        self.id = id
        self.name = name
        self.time = time
        self.action = action
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UInt64.self, forKey: .id)
        name = try c.value(.name, or: "")
        time = try c.decode(Double.self, forKey: .time)
        action = try c.decode(ActionDoc.self, forKey: .action)
    }
}

// MARK: - Status (from sbj_session_status_json)

struct OutputState: Decodable, Equatable {
    var name: String
    var status: String  // "running" | "on_backup" | "offline"
    var deviceId: String?
    var deviceName: String?
    var sampleRate: Int
    var channels: Int
    var xruns: UInt64
    var error: String?

    var isUp: Bool { status != "offline" }
}

struct CueProblem: Decodable, Equatable {
    var slot: Int
    var message: String
}

struct SessionStatus: Decodable, Equatable {
    var bank: Int
    /// "cue:<slot>", "stop_all", "next_bank", "prev_bank" or nil.
    var learning: String?
    var lastControl: String?
    /// nil when MIDI input is off.
    var midiInputs: [String]?
    var oscPort: Int?
    var dirty: Bool
    var path: String?
    var outputs: [OutputState]
    var problems: [CueProblem]
    var notices: [String]

    static func decode(_ json: String) throws -> SessionStatus {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(SessionStatus.self, from: Data(json.utf8))
    }
}

// MARK: - Pre-game check (from sbj_session_preflight_json)

struct PreflightCheck: Decodable, Equatable, Identifiable {
    var level: String  // "ok" | "info" | "warning" | "error"
    var message: String

    var id: String { level + message }
}

struct PreflightReport: Decodable, Equatable {
    var ready: Bool
    var memoryMb: Double
    var checks: [PreflightCheck]

    static func decode(_ json: String) throws -> PreflightReport {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(PreflightReport.self, from: Data(json.utf8))
    }
}
