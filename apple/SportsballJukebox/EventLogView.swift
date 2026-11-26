import AppKit
import SwiftUI

/// One entry from sbj_session_events_json.
struct EventEntry: Decodable, Identifiable, Equatable {
    var time: Double
    var level: String  // "info" | "warning" | "error"
    var kind: String  // "glitch" | "device" | "fire" | "show" | "notice"
    var message: String

    var id: String { "\(time)|\(message)" }
    var date: Date { Date(timeIntervalSince1970: time) }

    static func decodeList(_ json: String) -> [EventEntry] {
        (try? JSONDecoder().decode([EventEntry].self, from: Data(json.utf8))) ?? []
    }
}

/// The event log: every glitch (with what was playing), device problems,
/// switches to a backup device, failed fires and other warnings, for
/// working out what went wrong. Also saved in the log folder.
struct EventLogView: View {
    private enum Filter: String, CaseIterable {
        case all = "All"
        case problems = "Problems"
        case glitches = "Glitches"
    }

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var events: [EventEntry] = []
    @State private var filter = Filter.all
    @State private var copied = false

    private static let time: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .short
        f.timeStyle = .medium
        return f
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Event Log").font(.title2.bold())
                Spacer()
                Button {
                    refresh()
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
            }

            HStack {
                Picker("", selection: $filter) {
                    ForEach(Filter.allCases, id: \.self) { filter in
                        Text(filter.rawValue).tag(filter)
                    }
                }
                .pickerStyle(.segmented)
                .frame(width: 300)
                Spacer()
                Text(glitchSummary)
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(.secondary)
            }

            if shown.isEmpty {
                Text(filter == .all ? "Nothing logged yet." : "Nothing to show. All good.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(shown.reversed()) { event in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Image(systemName: icon(event))
                            .foregroundStyle(colour(event))
                            .frame(width: 16)
                        Text(Self.time.string(from: event.date))
                            .font(.callout.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .frame(width: 150, alignment: .leading)
                        Text(event.message)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }

            Text("Glitches are moments the audio couldn't keep up. Usually the Mac was busy (another app, sleep, a slow USB hub) or the buffer is too small. Newest first; the last 500 events are kept here and every event is saved in the log folder.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Button("Reset Glitch Count") {
                    model.resetGlitches()
                    refresh()
                }
                .disabled(totalGlitches == 0)
                Button("Open Log Folder") { model.openLogFolder() }
                    .help("Events are saved here, one file per day")
                Spacer()
                Button(copied ? "Copied" : "Copy Diagnostics") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(model.diagnosticsReport(events: events), forType: .string)
                    copied = true
                }
                .help("Copies details of this Mac, its audio devices and recent events, to paste into a bug report")
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 820, height: 560)
        .onAppear(perform: refresh)
    }

    private var shown: [EventEntry] {
        switch filter {
        case .all: return events
        case .problems: return events.filter { $0.level != "info" }
        case .glitches: return events.filter { $0.kind == "glitch" && $0.level != "info" }
        }
    }

    private var totalGlitches: UInt64 {
        (model.status?.outputs ?? []).reduce(0) { $0 + $1.xruns }
    }

    private var glitchSummary: String {
        let n = totalGlitches
        return n == 0 ? "No glitches" : "\(n) glitch\(n == 1 ? "" : "es") since the last reset"
    }

    private func icon(_ event: EventEntry) -> String {
        switch event.level {
        case "error": return "xmark.octagon.fill"
        case "warning": return event.kind == "glitch" ? "waveform.badge.exclamationmark" : "exclamationmark.triangle.fill"
        default: return "info.circle"
        }
    }

    private func colour(_ event: EventEntry) -> Color {
        switch event.level {
        case "error": return .red
        case "warning": return .orange
        default: return .secondary
        }
    }

    private func refresh() {
        events = model.events()
        copied = false
    }
}
