import AppKit
import SwiftUI

/// The show's watch folders: new audio files in them are added to the
/// library automatically (e.g. clips exported during the game).
struct WatchFoldersView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Watch Folders").font(.headline)
            Text("New audio files saved or copied into these folders are added to the library within a few seconds, once they've finished copying. Saved with the show.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if model.show.watchFolders.isEmpty {
                Text("No folders watched.")
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 6)
            } else {
                ForEach(model.show.watchFolders, id: \.path) { folder in
                    WatchFolderRow(folder: folder)
                }
            }

            HStack {
                Button {
                    model.addWatchFolder()
                } label: {
                    Label("Add Folder…", systemImage: "plus")
                }
                Spacer()
            }
            Text("Files stay where they are. Manage them in the Finder: a file removed from the library comes back while it's still in a watched folder.")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .frame(width: 380)
    }
}

private struct WatchFolderRow: View {
    @EnvironmentObject private var model: AppModel
    let folder: WatchFolder

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: missing ? "exclamationmark.triangle.fill" : "folder.fill")
                .foregroundStyle(missing ? Color.orange : Color.accentColor)
            VStack(alignment: .leading, spacing: 2) {
                Text(URL(fileURLWithPath: folder.path).lastPathComponent)
                    .lineLimit(1)
                Text(missing ? "Can't be found: \(folder.path)" : folder.path)
                    .font(.caption)
                    .foregroundStyle(missing ? Color.orange : Color.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Toggle("Include subfolders", isOn: subfolders)
                    .font(.caption)
                    .controlSize(.small)
            }
            Spacer(minLength: 0)
            Button {
                NSWorkspace.shared.open(URL(fileURLWithPath: folder.path))
            } label: {
                Image(systemName: "arrow.up.forward.square")
            }
            .buttonStyle(.borderless)
            .disabled(missing)
            .help("Show in Finder")
            Button {
                model.setWatchFolders(model.show.watchFolders.filter { $0.path != folder.path })
            } label: {
                Image(systemName: "minus.circle")
            }
            .buttonStyle(.borderless)
            .help("Stop watching (files already added stay in the library)")
        }
        .padding(.vertical, 2)
    }

    private var missing: Bool { model.isWatchFolderMissing(folder.path) }

    private var subfolders: Binding<Bool> {
        Binding(
            get: { folder.subfolders },
            set: { value in
                let folders = model.show.watchFolders.map { f -> WatchFolder in
                    var f = f
                    if f.path == folder.path { f.subfolders = value }
                    return f
                }
                model.setWatchFolders(folders)
            }
        )
    }
}
