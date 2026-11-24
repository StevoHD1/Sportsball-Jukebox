import SwiftUI

@main
struct SportsballJukeboxApp: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        Window("Sportsball Jukebox", id: "main") {
            ContentView()
                .environmentObject(model)
        }
        .defaultSize(width: 1100, height: 640)
    }
}
