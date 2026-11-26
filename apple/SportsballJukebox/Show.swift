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
    /// Seconds Stop All takes to fade everything out.
    var stopAllFade: Double = 0.5
    /// Output index used for previewing pads (e.g. headphones).
    var previewOutput: Int?
    /// Target loudness (LUFS) every pad is matched to; nil = off.
    var levelMatch: Double?
    /// The game's running order, fired step by step with GO.
    var rundown: [RundownItem] = []
    /// Audio files kept with the show (besides those pads use).
    var library: [String] = []
    /// Settings for new pads; nil = built-in.
    var padDefaults: PadDefaults?
    /// Banks (0-based) with Multiplay off.
    var singlePlay: [Int] = []
    /// Named colour presets kept with the show.
    var colors: [ShowColor] = []
    /// Colour played pads show in; nil = they keep their colour (✓ only).
    var playedColor: String?
    /// With a played colour, firing a pad marks it as played.
    var markPlayed = true
    /// Folders whose new audio files are added to the library.
    var watchFolders: [WatchFolder] = []
    /// Banks with their own settings for new pads.
    var bankDefaults: [BankDefaults] = []
    /// Where the library's player plays; nil = the preview output.
    var libraryOutput: Int?

    /// What new pads on `bank` get: the bank's own settings, else the
    /// show's.
    func defaults(forBank bank: Int) -> PadDefaults {
        bankDefaults.first { $0.bank == bank }?.defaults ?? padDefaults ?? PadDefaults()
    }

    func hasOwnDefaults(bank: Int) -> Bool {
        bankDefaults.contains { $0.bank == bank }
    }

    /// The preset with this colour value, if any.
    func preset(for value: String?) -> ShowColor? {
        guard let value else { return nil }
        return colors.first { $0.value.caseInsensitiveCompare(value) == .orderedSame }
    }

    enum CodingKeys: String, CodingKey {
        case format, version, name, slots, columns, banks, outputs, cues, controls
        case groupFade = "group_fade"
        case stopAllFade = "stop_all_fade"
        case previewOutput = "preview_output"
        case levelMatch = "level_match"
        case rundown, library
        case padDefaults = "pad_defaults"
        case singlePlay = "single_play"
        case colors
        case playedColor = "played_color"
        case markPlayed = "mark_played"
        case watchFolders = "watch_folders"
        case bankDefaults = "bank_defaults"
        case libraryOutput = "library_output"
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
        stopAllFade = try c.value(.stopAllFade, or: stopAllFade)
        previewOutput = try c.decodeIfPresent(Int.self, forKey: .previewOutput)
        levelMatch = try c.decodeIfPresent(Double.self, forKey: .levelMatch)
        rundown = try c.value(.rundown, or: [])
        library = try c.value(.library, or: [])
        padDefaults = try c.decodeIfPresent(PadDefaults.self, forKey: .padDefaults)
        singlePlay = try c.value(.singlePlay, or: [])
        colors = try c.value(.colors, or: [])
        playedColor = try c.decodeIfPresent(String.self, forKey: .playedColor)
        markPlayed = try c.value(.markPlayed, or: true)
        watchFolders = try c.value(.watchFolders, or: [])
        bankDefaults = try c.value(.bankDefaults, or: [])
        libraryOutput = try c.decodeIfPresent(Int.self, forKey: .libraryOutput)
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
    var rundownGo: MidiTrigger?
    var fadeLast: MidiTrigger?

    enum CodingKeys: String, CodingKey {
        case stopAll = "stop_all"
        case nextBank = "next_bank"
        case prevBank = "prev_bank"
        case rundownGo = "rundown_go"
        case fadeLast = "fade_last"
    }
}

/// A named pad colour kept with the show.
/// A folder whose new audio files join the library automatically.
struct WatchFolder: Codable, Equatable, Hashable {
    var path: String
    /// Also watch the folders inside it.
    var subfolders = true
    /// Download files that are only in the cloud (Box Drive, iCloud).
    var download = true

    init(path: String, subfolders: Bool = true, download: Bool = true) {
        self.path = path
        self.subfolders = subfolders
        self.download = download
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        path = try c.decode(String.self, forKey: .path)
        subfolders = try c.decodeIfPresent(Bool.self, forKey: .subfolders) ?? true
        download = try c.decodeIfPresent(Bool.self, forKey: .download) ?? true
    }
}

/// A bank's own settings for new pads.
struct BankDefaults: Codable, Equatable {
    var bank: Int
    var defaults: PadDefaults
}

struct ShowColor: Codable, Equatable, Hashable {
    var name: String
    /// "#RRGGBB".
    var value: String
}

/// How a pad treats its bank's Multiplay setting.
enum PlayMode: String, Codable, CaseIterable {
    case bank, over, solo

    var label: String {
        switch self {
        case .bank: return "Follow the bank's Multiplay"
        case .over: return "Play over everything"
        case .solo: return "Solo: fade out the bank's other pads"
        }
    }
}

enum FireActionKind: String, Codable, CaseIterable {
    case stopOthers = "stop_others"
    case firePad = "fire_pad"
    case stopPad = "stop_pad"
    case duck
    case bank

    var label: String {
        switch self {
        case .stopOthers: return "Stop other pads"
        case .firePad: return "Fire a pad"
        case .stopPad: return "Stop a pad"
        case .duck: return "Duck everything else"
        case .bank: return "Switch to bank"
        }
    }
}

/// One action a pad runs when it starts (the show file's `on_fire`).
struct FireAction: Codable, Equatable, Identifiable {
    var kind: FireActionKind
    var pad: Int?
    var seconds: Double?
    var db: Double?
    var bank: Int?
    /// Stop other pads: only this bank's.
    var thisBank: Bool?
    /// For SwiftUI lists only (not saved).
    var id = UUID()

    enum CodingKeys: String, CodingKey {
        case kind, pad, seconds, db, bank
        case thisBank = "this_bank"
    }

    init(kind: FireActionKind) {
        self.kind = kind
        switch kind {
        case .stopOthers: seconds = 1
        case .duck:
            db = -12
            seconds = 5
        default: break
        }
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(FireActionKind.self, forKey: .kind)
        pad = try c.decodeIfPresent(Int.self, forKey: .pad)
        seconds = try c.decodeIfPresent(Double.self, forKey: .seconds)
        db = try c.decodeIfPresent(Double.self, forKey: .db)
        bank = try c.decodeIfPresent(Int.self, forKey: .bank)
        thisBank = try c.decodeIfPresent(Bool.self, forKey: .thisBank)
    }

    static func == (a: FireAction, b: FireAction) -> Bool {
        a.kind == b.kind && a.pad == b.pad && a.seconds == b.seconds && a.db == b.db
            && a.bank == b.bank && (a.thisBank ?? false) == (b.thisBank ?? false)
    }

    /// e.g. "Fire \"Horn\" after 2 s".
    func describe(in show: ShowDoc) -> String {
        let padName = pad.map { slot -> String in
            guard let cue = show.cue(at: slot) else { return "an empty pad" }
            return cue.title.isEmpty ? "pad \(slot % max(show.slots, 1) + 1)" : "\"\(cue.title)\""
        } ?? "(no pad)"
        switch kind {
        case .stopOthers:
            let scope = thisBank == true ? "this bank's other pads" : "all other pads"
            return "Stop \(scope)" + (seconds.map { $0 > 0 ? String(format: " over %.1f s", $0) : " at once" } ?? "")
        case .firePad:
            let delay = (seconds ?? 0) > 0 ? String(format: " after %.1f s", seconds ?? 0) : ""
            return "Fire \(padName)\(delay)"
        case .stopPad:
            return "Stop \(padName)" + (seconds.map { String(format: " over %.1f s", $0) } ?? "")
        case .duck:
            return String(format: "Duck everything else %.0f dB for %.1f s", db ?? -12, seconds ?? 5)
        case .bank:
            let index = bank ?? 0
            let name = show.banks.indices.contains(index) ? show.banks[index] : "bank \(index + 1)"
            return "Switch to \(name)"
        }
    }
}

/// Settings given to new pads when audio is dropped on an empty pad.
struct PadDefaults: Codable, Equatable {
    var color: String?
    var fadeIn: Double = 0
    var fadeOut: Double?
    var loop = false
    var group: String?
    var output = 0
    var duck: Float?
    /// Start new pads where the sound starts.
    var trimSilence = true
    var levelMatch = true

    enum CodingKeys: String, CodingKey {
        case color, loop, group, output, duck
        case fadeIn = "fade_in"
        case fadeOut = "fade_out"
        case trimSilence = "trim_silence"
        case levelMatch = "level_match"
    }

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        color = try c.decodeIfPresent(String.self, forKey: .color)
        fadeIn = try c.value(.fadeIn, or: 0)
        fadeOut = try c.decodeIfPresent(Double.self, forKey: .fadeOut)
        loop = try c.value(.loop, or: false)
        group = try c.decodeIfPresent(String.self, forKey: .group)
        output = try c.value(.output, or: 0)
        duck = try c.decodeIfPresent(Float.self, forKey: .duck)
        trimSilence = try c.value(.trimSilence, or: true)
        levelMatch = try c.value(.levelMatch, or: true)
    }

    /// Applies these settings to a new pad.
    func apply(to cue: inout CueDoc, outputs: Int) {
        cue.color = color
        cue.fadeIn = fadeIn
        cue.fadeOut = fadeOut
        cue.loop = loop
        cue.group = group
        cue.output = output < outputs ? output : 0
        cue.duck = duck
        cue.levelMatch = levelMatch
    }
}

/// One file in the show's audio library (from sbj_session_library_json).
struct LibraryEntry: Decodable, Equatable, Identifiable {
    var file: String
    /// Slots of the pads that use it.
    var usedBy: [Int]
    /// Decoded, so duration and loudness are known.
    var loaded: Bool
    var duration: Double?
    var loudness: Double?
    var error: String?
    /// Only in the cloud (Box Drive, iCloud), not on this Mac yet.
    var onlineOnly: Bool
    var downloading: Bool

    var id: String { file }
    var name: String { URL(fileURLWithPath: file).deletingPathExtension().lastPathComponent }

    static func decodeList(_ json: String) -> [LibraryEntry] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return (try? decoder.decode([LibraryEntry].self, from: Data(json.utf8))) ?? []
    }
}

/// One step of the game rundown: a pad to fire, or a note when `pad` is
/// nil.
struct RundownItem: Codable, Equatable, Identifiable {
    var id: UInt64
    /// Slot of the pad this step fires.
    var pad: Int?
    /// A note, or a label shown instead of the pad's title.
    var text = ""

    enum CodingKeys: String, CodingKey { case id, pad, text }

    init(id: UInt64, pad: Int? = nil, text: String = "") {
        self.id = id
        self.pad = pad
        self.text = text
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UInt64.self, forKey: .id)
        pad = try c.decodeIfPresent(Int.self, forKey: .pad)
        text = try c.value(.text, or: "")
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
    /// Picture shown on the pad (a path; found next to the show or in its
    /// Images folder if moved).
    var image: String?
    var clips: [ClipDoc] = []
    var markers: [MarkerDoc] = []
    var midi: MidiTrigger?
    /// Exclusive group: firing this fades out others in the same group.
    var group: String?
    /// dB (negative) to lower other pads on the same output while playing.
    var duck: Float?
    var retrigger: Retrigger = .restart
    var route: RouteDoc?
    /// Seconds to fade in each time the pad fires.
    var fadeIn: Double = 0
    /// Seconds this pad's fade-outs take (nil = 0.5).
    var fadeOut: Double?
    /// Makes the pad a playlist.
    var playlist: PlaylistDoc?
    /// Sponsor credited when this pad plays (sponsor reports).
    var sponsor: String?
    /// Follows the show's level matching (when it is on).
    var levelMatch = true
    /// How the pad treats its bank's Multiplay setting.
    var playMode: PlayMode = .bank
    /// Actions run, in order, each time the pad starts.
    var onFire: [FireAction] = []

    /// Has a play mode or actions worth flagging on the pad.
    var hasActions: Bool { playMode != .bank || !onFire.isEmpty }

    enum CodingKeys: String, CodingKey {
        case slot, title, hotkey, output, gain, loop, color, image, clips, markers, midi
        case group, duck, retrigger, route, playlist, sponsor
        case fadeIn = "fade_in"
        case fadeOut = "fade_out"
        case levelMatch = "level_match"
        case playMode = "play_mode"
        case onFire = "on_fire"
    }

    var fadeOutSeconds: Double { fadeOut ?? 0.5 }

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
        image = try c.decodeIfPresent(String.self, forKey: .image)
        clips = try c.value(.clips, or: [])
        markers = try c.value(.markers, or: [])
        midi = try c.decodeIfPresent(MidiTrigger.self, forKey: .midi)
        group = try c.decodeIfPresent(String.self, forKey: .group)
        duck = try c.decodeIfPresent(Float.self, forKey: .duck)
        retrigger = try c.value(.retrigger, or: .restart)
        route = try c.decodeIfPresent(RouteDoc.self, forKey: .route)
        fadeIn = try c.value(.fadeIn, or: 0)
        fadeOut = try c.decodeIfPresent(Double.self, forKey: .fadeOut)
        playlist = try c.decodeIfPresent(PlaylistDoc.self, forKey: .playlist)
        sponsor = try c.decodeIfPresent(String.self, forKey: .sponsor)
        levelMatch = try c.value(.levelMatch, or: true)
        playMode = try c.value(.playMode, or: .bank)
        onFire = try c.value(.onFire, or: [])
    }

    /// A playlist pad of the given files.
    init(slot: Int, title: String, playlist files: [String]) {
        self.slot = slot
        self.title = title
        self.playlist = PlaylistDoc(items: files)
    }

    /// Smallest marker id not in use.
    var nextMarkerID: UInt64 {
        (markers.map(\.id).max() ?? 0) + 1
    }
}

struct PlaylistDoc: Codable, Equatable {
    var items: [String]
    var shuffle = false
    var autoAdvance = true
    var crossfade: Double = 2

    enum CodingKeys: String, CodingKey {
        case items, shuffle, crossfade
        case autoAdvance = "auto_advance"
    }

    init(items: [String]) {
        self.items = items
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        items = try c.value(.items, or: [])
        shuffle = try c.value(.shuffle, or: false)
        autoAdvance = try c.value(.autoAdvance, or: true)
        crossfade = try c.value(.crossfade, or: 2)
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
    case fire
    case stopPad = "stop_pad"
    case volume

    var label: String {
        switch self {
        case .jump: return "Jump"
        case .loop: return "Loop"
        case .pause: return "Pause"
        case .stop: return "Stop"
        case .fadeOut: return "Fade Out"
        case .fire: return "Fire Pad"
        case .stopPad: return "Stop Pad"
        case .volume: return "Volume"
        }
    }

    var needsTarget: Bool { self == .jump || self == .loop }
    /// Acts on another pad.
    var needsPad: Bool { self == .fire || self == .stopPad }
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
    /// Fire / Stop Pad: the target pad's slot.
    var pad: Int? = nil
    /// Volume: level in dB (0 = as set on the pad).
    var db: Double? = nil
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

// MARK: - Sound analysis (from sbj_session_sound_info_json)

struct SoundInfo: Decodable, Equatable {
    var duration: Double
    /// Integrated loudness, LUFS (nil for silence).
    var loudness: Double?
    var peakDb: Double?
    var leadSilence: Double
    var tailSilence: Double
    /// What the show's level matching applies (nil when off).
    var levelGainDb: Double?

    static func decode(_ json: String) -> SoundInfo? {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(SoundInfo.self, from: Data(json.utf8))
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

struct PreviewInfo: Decodable, Equatable {
    /// The pad, or 4294967295 when a library file is previewed.
    var slot: Int
    /// The library file being previewed, if any.
    var file: String?
    var playing: Bool
    /// Paused (play carries on from here).
    var paused: Bool
    var position: Double
    var duration: Double
}

struct SessionStatus: Decodable, Equatable {
    var game: String
    /// Rundown step GO fires next; nil when finished.
    var standby: Int?
    /// The most recently fired pad still playing (Fade Last / Stop Last).
    var last: Int?
    /// Banks with Multiplay off.
    var singlePlay: [Int]
    /// Pads marked as played this game.
    var played: [Int]
    /// Counts imports from watch folders (reload the library on change).
    var watchImports: UInt64
    /// Changes as cloud downloads start and finish.
    var downloads: UInt64
    /// Watch folders that can't be read right now.
    var missingWatchFolders: [String]
    var previewOutput: Int?
    var preview: PreviewInfo?
    var bank: Int
    /// "cue:<slot>", "stop_all", "next_bank", "prev_bank" or nil.
    var learning: String?
    var lastControl: String?
    /// nil when MIDI input is off.
    var midiInputs: [String]?
    var oscPort: Int?
    /// nil when the phone/tablet remote is off.
    var remotePort: Int?
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

// MARK: - Play log (from sbj_session_log_json)

struct LogEntry: Decodable, Equatable {
    var started: Double
    var seconds: Double
    var game: String
    var show: String
    var slot: Int
    var bank: Int
    var pad: Int
    var title: String
    var file: String
    var sponsor: String?
    var source: String
    var output: String
    var open: Bool

    var startDate: Date { Date(timeIntervalSince1970: started) }
    var fileName: String {
        URL(fileURLWithPath: file).deletingPathExtension().lastPathComponent
    }
}

struct LogTotal: Decodable, Equatable {
    var name: String
    var plays: Int
    var seconds: Double
}

struct PlayLog: Decodable, Equatable {
    var entries: [LogEntry]
    var songs: [LogTotal]
    var sponsors: [LogTotal]

    static let empty = PlayLog(entries: [], songs: [], sponsors: [])

    static func decode(_ json: String) throws -> PlayLog {
        try JSONDecoder().decode(PlayLog.self, from: Data(json.utf8))
    }
}
