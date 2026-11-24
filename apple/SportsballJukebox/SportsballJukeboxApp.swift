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
                Button("MIDI & OSC…") { model.showingControls = true }
                    .keyboardShortcut("m", modifiers: [.command, .shift])
                Button("Show Settings…") { model.showingSettings = true }
                Button(model.showingNowPlaying ? "Hide Now Playing" : "Show Now Playing") {
                    model.showingNowPlaying.toggle()
                }
                .keyboardShortcut("p", modifiers: [.command, .shift])
                Button("Game Log…") { model.showingLog = true }
                    .keyboardShortcut("l", modifiers: [.command, .shift])
                Button("Pre-game Check…") {
                    model.runPreflight()
                    model.showingPreflight = true
                }
                .keyboardShortcut("k", modifiers: [.command, .shift])
                Divider()
                ForEach(Array(model.show.banks.prefix(9).enumerated()), id: \.offset) { index, name in
                    Button("Bank: \(name)") { model.selectBank(index) }
                        .keyboardShortcut(KeyEquivalent(Character(String(index + 1))), modifiers: [.command])
                }
                Divider()
                Button("Stop All") { model.stopAll() }
                    .keyboardShortcut(".", modifiers: [.command])
            }
        }
    }
}
