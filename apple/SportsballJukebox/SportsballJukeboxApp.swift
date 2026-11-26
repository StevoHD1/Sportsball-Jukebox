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
                Button("Save With Audio…") { model.saveWithAudio() }
                    .keyboardShortcut("s", modifiers: [.command, .option])
            }
            CommandMenu("Show") {
                Button("Audio Outputs…") { model.showingOutputs = true }
                    .keyboardShortcut("u", modifiers: [.command, .shift])
                Button("Control (MIDI, OSC, Remote)…") { model.showingControls = true }
                    .keyboardShortcut("m", modifiers: [.command, .shift])
                Button("Show Settings…") { model.showingSettings = true }
                Button("Colour Presets…") { model.showingColours = true }
                Button("Reset Played on This Page") { model.resetPlayed(bank: model.currentBank) }
                Button("Reset Played on All Pages") { model.resetPlayed(bank: nil) }
                Button(model.showingLibrary ? "Hide Library" : "Show Library") {
                    model.showingLibrary.toggle()
                }
                .keyboardShortcut("l", modifiers: [.command, .option])
                Button("Add to Library…") { model.importToLibrary() }
                Button(model.showingRundown ? "Hide Rundown" : "Show Rundown") {
                    model.showingRundown.toggle()
                }
                .keyboardShortcut("r", modifiers: [.command, .shift])
                Button("Rundown: Next Step") { model.stepStandby(forward: true) }
                    .keyboardShortcut(.rightArrow, modifiers: [.command])
                Button("Rundown: Previous Step") { model.stepStandby(forward: false) }
                    .keyboardShortcut(.leftArrow, modifiers: [.command])
                Button(model.showingNowPlaying ? "Hide Now Playing" : "Show Now Playing") {
                    model.showingNowPlaying.toggle()
                }
                .keyboardShortcut("p", modifiers: [.command, .shift])
                Button(model.nowPlayingOnTop ? "Now Playing on the Right" : "Now Playing Across the Top") {
                    model.nowPlayingOnTop.toggle()
                }
                Button(model.fitPadsToWindow ? "Pads: Fixed Size" : "Pads: Fit to Window") {
                    model.fitPadsToWindow.toggle()
                }
                Button("Bigger Pads") { model.changePadSize(by: 10) }
                    .keyboardShortcut("=", modifiers: [.command])
                Button("Smaller Pads") { model.changePadSize(by: -10) }
                    .keyboardShortcut("-", modifiers: [.command])
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
