import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The show's audio library: every file kept with the show and every file
/// a pad uses. Drag a file onto an empty pad to make a pad, onto a pad's
/// timeline (in its editor) to add a clip, or preview it in the
/// headphones.
struct LibraryView: View {
    @EnvironmentObject private var model: AppModel
    var width: Double = 290
    @State private var search = ""
    @State private var unusedOnly = false
    @FocusState private var searching: Bool
    @State private var isDropTarget = false
    @State private var selection: Set<String> = []

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Library").font(.headline)
                Spacer()
                Button {
                    model.showingWatchFolders.toggle()
                } label: {
                    Label("Watch Folders…", systemImage: watchIcon)
                }
                .labelStyle(.iconOnly)
                .foregroundStyle(watchMissing ? Color.orange : Color.primary)
                .help(watchHelp)
                .popover(isPresented: $model.showingWatchFolders, arrowEdge: .bottom) {
                    WatchFoldersView()
                        .environmentObject(model)
                }
                Button {
                    model.importToLibrary()
                } label: {
                    Label("Add Files or Folders…", systemImage: "plus")
                }
                .labelStyle(.iconOnly)
                .help("Add audio files or whole folders to the library")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)

            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search", text: $search)
                    .textFieldStyle(.plain)
                    .focused($searching)
                if !search.isEmpty {
                    Button {
                        search = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                }
            }
            .padding(6)
            .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
            .padding(.horizontal, 12)

            Toggle("Only files no pad uses", isOn: $unusedOnly)
                .font(.caption)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)

            Divider()

            if model.library.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "music.note.list")
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                    Text("Drop audio files or folders here, or use +. Then drag them onto pads, or into a pad's timeline in its editor.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                .padding(20)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(entries, selection: $selection) { entry in
                    LibraryRow(entry: entry, selection: selection)
                }
                .listStyle(.sidebar)
                .onKeyPress(.return) {
                    guard !selection.isEmpty else { return .ignored }
                    model.addLibraryFilesToBank(selectedFiles)
                    return .handled
                }
                .onKeyPress(.space) {
                    guard let file = playerFile else { return .ignored }
                    if model.libraryPlaying?.file == file, model.libraryPlaying?.playing == true {
                        model.pauseLibrary()
                    } else {
                        model.playLibraryFile(file)
                    }
                    return .handled
                }

                LibraryTransport(file: playerFile, selected: selectedFiles)
            }

            Text(summary)
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(8)
        }
        .frame(width: width)
        .overlay {
            if isDropTarget {
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(Color.accentColor, lineWidth: 3)
                    .padding(2)
            }
        }
        .dropDestination(for: URL.self) { urls, _ in
            let files = urls.filter(\.isFileURL)
            guard !files.isEmpty else { return false }
            model.addToLibrary(urls: files)
            return true
        } isTargeted: { isDropTarget = $0 }
        // Pad keys would otherwise fire while typing a search.
        .onChange(of: searching) { _, focused in
            model.suspendHotkeys = focused
        }
        .onDisappear { model.suspendHotkeys = false }
    }

    /// Selected files in list order.
    private var selectedFiles: [String] {
        entries.map(\.file).filter { selection.contains($0) }
    }

    /// The file the transport plays: the one loaded, else the selection.
    private var playerFile: String? {
        if let file = model.libraryPlaying?.file,
           selection.isEmpty || selection.contains(file) {
            return file
        }
        return selectedFiles.count == 1 ? selectedFiles.first : model.libraryPlaying?.file
    }

    private var watchMissing: Bool {
        !(model.status?.missingWatchFolders.isEmpty ?? true)
    }

    private var watchIcon: String {
        if watchMissing { return "exclamationmark.triangle" }
        return model.show.watchFolders.isEmpty ? "eye" : "eye.fill"
    }

    private var watchHelp: String {
        let count = model.show.watchFolders.count
        if watchMissing { return "A watch folder can't be found" }
        if count == 0 { return "Watch folders: add new audio files to the library automatically" }
        return "Watching \(count) folder\(count == 1 ? "" : "s") for new audio files"
    }

    private var entries: [LibraryEntry] {
        let query = search.trimmingCharacters(in: .whitespaces).lowercased()
        return model.library.filter { entry in
            (query.isEmpty || entry.name.lowercased().contains(query))
                && (!unusedOnly || entry.usedBy.isEmpty)
        }
    }

    private var summary: String {
        let total = model.library.count
        let unused = model.library.filter(\.usedBy.isEmpty).count
        return "\(total) file\(total == 1 ? "" : "s") · \(unused) not on a pad"
    }
}

private struct LibraryRow: View {
    @EnvironmentObject private var model: AppModel
    let entry: LibraryEntry
    var selection: Set<String> = []

    var body: some View {
        HStack(spacing: 8) {
            Button {
                model.previewFile(entry.file)
            } label: {
                if model.loadingPreview == entry.file {
                    ProgressView().controlSize(.small)
                } else {
                    Image(systemName: isPreviewing ? "stop.fill" : "headphones")
                }
            }
            .buttonStyle(.borderless)
            .frame(width: 18)
            .help(isPreviewing ? "Stop the preview" : "Preview in the headphones")

            VStack(alignment: .leading, spacing: 1) {
                Text(entry.name)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(detail)
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(entry.error == nil ? Color.secondary : Color.orange)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            if entry.downloading {
                ProgressView()
                    .controlSize(.mini)
                    .help("Downloading from the cloud")
            } else if entry.onlineOnly {
                Button {
                    model.downloadFile(entry.file)
                } label: {
                    Image(systemName: "icloud.and.arrow.down")
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .help("Only in the cloud (Box Drive, iCloud): click to download it to this Mac")
            }
            if !entry.usedBy.isEmpty {
                Text("\(entry.usedBy.count)")
                    .font(.caption2.weight(.semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(Color.accentColor.opacity(0.25), in: Capsule())
                    .help("Used by \(padNames)")
            }
        }
        .contentShape(Rectangle())
        .draggable(model.libraryURL(entry.file)) {
            Label(entry.name, systemImage: "waveform")
                .padding(8)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
        }
        .contextMenu {
            Button(isPreviewing ? "Stop Preview" : "Preview in Headphones") { model.previewFile(entry.file) }
            Button(addTitle) { model.addLibraryFilesToBank(addFiles) }
            if entry.onlineOnly {
                Button("Download from Cloud") { model.downloadFile(entry.file) }
            }
            Button("Show in Finder") {
                NSWorkspace.shared.activateFileViewerSelecting([model.libraryURL(entry.file)])
            }
            Divider()
            Button("Remove from Library", role: .destructive) { model.removeFromLibrary(entry.file) }
                .disabled(!entry.usedBy.isEmpty)
        }
        .accessibilityElement(children: .combine)
        .accessibilityHint("Drag onto a pad to use it")
    }

    /// The right-clicked file, or the whole selection if it's in it.
    private var addFiles: [String] {
        selection.contains(entry.file) && selection.count > 1
            ? model.library.map(\.file).filter { selection.contains($0) }
            : [entry.file]
    }

    private var addTitle: String {
        let count = addFiles.count
        return count == 1 ? "Add to This Bank" : "Add \(count) Files to This Bank"
    }

    private var isPreviewing: Bool {
        model.status?.preview?.file == entry.file && model.status?.preview?.playing == true
    }

    private var detail: String {
        if let error = entry.error { return error }
        var parts: [String] = []
        if let duration = entry.duration { parts.append(formatTime(duration)) }
        if let loudness = entry.loudness { parts.append(String(format: "%.0f LUFS", loudness)) }
        if parts.isEmpty { parts.append(URL(fileURLWithPath: entry.file).pathExtension.uppercased()) }
        if entry.usedBy.isEmpty { parts.append("not on a pad") }
        return parts.joined(separator: " · ")
    }

    private var padNames: String {
        entry.usedBy.map { slot in
            let title = model.show.cue(at: slot)?.title ?? ""
            return title.isEmpty ? "pad \(slot % max(model.show.slots, 1) + 1)" : "\"\(title)\""
        }
        .joined(separator: ", ")
    }
}

/// The library's player: play / pause / stop the selected file, scrub,
/// choose where it plays, and add the selection to the bank shown.
private struct LibraryTransport: View {
    @EnvironmentObject private var model: AppModel
    let file: String?
    let selected: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider()
            HStack(spacing: 6) {
                Text(title)
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 4)
                Menu {
                    Button {
                        model.setLibraryOutput(nil)
                    } label: {
                        if model.show.libraryOutput == nil { Image(systemName: "checkmark") }
                        Text(previewLabel)
                    }
                    Divider()
                    ForEach(Array(model.show.outputs.enumerated()), id: \.offset) { index, output in
                        Button {
                            model.setLibraryOutput(index)
                        } label: {
                            if model.show.libraryOutput == index { Image(systemName: "checkmark") }
                            Text(output.name)
                        }
                    }
                } label: {
                    Label(model.libraryOutputName, systemImage: model.show.libraryOutput == nil ? "headphones" : "speaker.wave.2")
                        .font(.caption)
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .help("Where library files play: the preview output (headphones) unless you pick another")
            }

            ScrubBar(
                progress: progress,
                duration: loaded?.duration ?? 0,
                tint: model.show.libraryOutput == nil ? .blue : .orange
            ) {
                model.seekPreview(to: $0)
            }
            .disabled(loaded == nil)

            HStack(spacing: 10) {
                Text(formatTime(loaded?.position ?? 0))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                Spacer()
                Button {
                    guard let file else { return }
                    if isPlaying {
                        model.pauseLibrary()
                    } else {
                        model.playLibraryFile(file)
                    }
                } label: {
                    if let file, model.loadingPreview == file {
                        ProgressView().controlSize(.small)
                    } else {
                        Image(systemName: isPlaying ? "pause.fill" : "play.fill")
                    }
                }
                .disabled(file == nil)
                .help(isPlaying ? "Pause (Space)" : "Play (Space)")
                Button {
                    model.stopPreview()
                } label: {
                    Image(systemName: "stop.fill")
                }
                .disabled(loaded == nil)
                .help("Stop")
                Spacer()
                Text("-" + formatTime(max((loaded?.duration ?? 0) - (loaded?.position ?? 0), 0)))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)
            .font(.title3)

            Button {
                model.addLibraryFilesToBank(selected)
            } label: {
                Label(addTitle, systemImage: "square.grid.3x3.square")
                    .frame(maxWidth: .infinity)
            }
            .disabled(selected.isEmpty)
            .help("Make a pad for each selected file on the bank shown (Return)")
        }
        .padding(.horizontal, 12)
        .padding(.top, 2)
    }

    /// The player's state, if it holds this file.
    private var loaded: PreviewInfo? {
        guard let playing = model.libraryPlaying, playing.file == file else { return nil }
        return playing
    }

    private var isPlaying: Bool { loaded?.playing == true }

    private var progress: Double {
        guard let loaded, loaded.duration > 0 else { return 0 }
        return loaded.position / loaded.duration
    }

    private var title: String {
        guard let file else { return selected.count > 1 ? "\(selected.count) files selected" : "Select a file" }
        return URL(fileURLWithPath: file).deletingPathExtension().lastPathComponent
    }

    private var previewLabel: String {
        if let index = model.show.previewOutput, model.show.outputs.indices.contains(index) {
            return "Preview Output (\(model.show.outputs[index].name))"
        }
        return "Preview Output (not set)"
    }

    private var addTitle: String {
        let bank = model.show.banks.indices.contains(model.currentBank) ? model.show.banks[model.currentBank] : "Bank"
        switch selected.count {
        case 0: return "Add to \(bank)"
        case 1: return "Add to \(bank)"
        default: return "Add \(selected.count) to \(bank)"
        }
    }
}
