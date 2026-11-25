import SwiftUI

/// What played, when and for how long, with totals per song (licensing)
/// and per sponsor, and CSV export.
struct GameLogView: View {
    private enum Tab: String, CaseIterable {
        case plays = "Plays"
        case songs = "Songs"
        case sponsors = "Sponsors"
    }

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var log = PlayLog.empty
    @State private var tab = Tab.plays
    @State private var game = ""
    @State private var confirmingClear = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Game Log").font(.title2.bold())
                Spacer()
                Button {
                    refresh()
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
            }

            HStack {
                Text("Game")
                TextField("e.g. vs. Tigers", text: $game)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { model.setGame(game) }
                    .frame(width: 260)
                Button("Set") { model.setGame(game) }
                Text("Stamped on everything played from now on.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Picker("", selection: $tab) {
                ForEach(Tab.allCases, id: \.self) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .frame(width: 320)

            switch tab {
            case .plays:
                PlaysList(entries: log.entries)
            case .songs:
                TotalsList(totals: log.songs, empty: "Nothing played yet.")
            case .sponsors:
                TotalsList(
                    totals: log.sponsors,
                    empty: "No sponsored pads played yet. Set a sponsor in a pad's editor."
                )
            }

            HStack {
                Button("Clear…") { confirmingClear = true }
                Button("Open Log Folder") { model.openLogFolder() }
                    .help("Every play is also saved here, one file per day")
                Spacer()
                Button("Export CSV…") { model.exportLog(log) }
                    .disabled(log.entries.isEmpty)
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 820, height: 560)
        .onAppear {
            game = model.game
            refresh()
        }
        .confirmationDialog(
            "Clear the log shown here?",
            isPresented: $confirmingClear
        ) {
            Button("Clear", role: .destructive) {
                model.clearLog()
                refresh()
            }
        } message: {
            Text("The daily log files in the log folder are kept.")
        }
    }

    private func refresh() {
        log = model.playLog()
    }
}

private struct PlaysList: View {
    let entries: [LogEntry]

    private static let time: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .short
        f.timeStyle = .medium
        return f
    }()

    var body: some View {
        if entries.isEmpty {
            Text("Nothing played yet.")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            List {
                HStack {
                    Text("Started").frame(width: 150, alignment: .leading)
                    Text("Pad").frame(maxWidth: .infinity, alignment: .leading)
                    Text("Song / file").frame(maxWidth: .infinity, alignment: .leading)
                    Text("Sponsor").frame(width: 110, alignment: .leading)
                    Text("By").frame(width: 70, alignment: .leading)
                    Text("Played").frame(width: 60, alignment: .trailing)
                }
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                ForEach(Array(entries.reversed().enumerated()), id: \.offset) { _, entry in
                    HStack {
                        Text(Self.time.string(from: entry.startDate))
                            .frame(width: 150, alignment: .leading)
                        Text(entry.title).lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Text(entry.fileName).lineLimit(1)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Text(entry.sponsor ?? "").lineLimit(1)
                            .frame(width: 110, alignment: .leading)
                        Text(entry.source).frame(width: 70, alignment: .leading)
                        Text(entry.open ? "playing" : formatTime(entry.seconds))
                            .font(.callout.monospacedDigit())
                            .frame(width: 60, alignment: .trailing)
                    }
                    .font(.callout)
                }
            }
        }
    }
}

private struct TotalsList: View {
    let totals: [LogTotal]
    let empty: String

    var body: some View {
        if totals.isEmpty {
            Text(empty)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            List {
                HStack {
                    Text("Name").frame(maxWidth: .infinity, alignment: .leading)
                    Text("Plays").frame(width: 60, alignment: .trailing)
                    Text("Total time").frame(width: 90, alignment: .trailing)
                }
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                ForEach(Array(totals.enumerated()), id: \.offset) { _, total in
                    HStack {
                        Text(total.name).lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Text("\(total.plays)")
                            .font(.callout.monospacedDigit())
                            .frame(width: 60, alignment: .trailing)
                        Text(formatTime(total.seconds))
                            .font(.callout.monospacedDigit())
                            .frame(width: 90, alignment: .trailing)
                    }
                }
            }
        }
    }
}
