import SwiftUI

@main
struct SportsballJukeboxApp: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        Window("Sportsball Jukebox", id: "main") {
            ContentView()
                .environmentObject(model)
        }
        .defaultSize(width: 1100, height: 680)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Show") { model.newShow() }
                    .keyboardShortcut("n")
                Button("Open Show…") { model.openShowPanel() }
                    .keyboardShortcut("o")
            }
            CommandGroup(replacing: .saveItem) {
                Button("Save") { model.save() }
                    .keyboardShortcut("s")
                Button("Save As…") { model.saveAs() }
                    .keyboardShortcut("s", modifiers: [.command, .shift])
            }
            CommandMenu("Show") {
                Button("Audio Outputs…") { model.showingOutputs = true }
                    .keyboardShortcut("u", modifiers: [.command, .shift])
                Button("Show Settings…") { model.showingSettings = true }
                Divider()
                Button("Stop All") { model.stopAll() }
                    .keyboardShortcut(".", modifiers: [.command])
            }
        }
    }
}
