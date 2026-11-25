import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The show's audio library: every file kept with the show and every file
/// a pad uses. Drag a file onto an empty pad to make a pad, onto a pad's
/// timeline (in its editor) to add a clip, or preview it in the
/// headphones.
struct LibraryView: View {
    @EnvironmentObject private var model: AppModel
    @State private var search = ""
    @State private var unusedOnly = false
    @FocusState private var searching: Bool
    @State private var isDropTarget = false

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Library").font(.headline)
                Spacer()
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
                List(entries) { entry in
                    LibraryRow(entry: entry)
                }
                .listStyle(.sidebar)
            }

            Text(summary)
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(8)
        }
        .frame(width: 290)
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
