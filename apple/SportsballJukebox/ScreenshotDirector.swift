import AppKit
import Foundation

/// Documentation/demo mode, used by CI to produce screenshots:
///
///     "Sportsball Jukebox" --open-show Demo.sbjshow --screenshots out/
///
/// Opens the show, fires a few pads, captures the main window, the pad
/// editor, the outputs sheet and the pre-game check as PNGs, then quits.
@MainActor
struct ScreenshotDirector {
    let show: URL
    let output: URL

    static func fromArguments() -> ScreenshotDirector? {
        let args = CommandLine.arguments
        func value(after flag: String) -> String? {
            guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
            return args[i + 1]
        }
        guard let show = value(after: "--open-show"), let out = value(after: "--screenshots") else {
            return nil
        }
        return ScreenshotDirector(
            show: URL(fileURLWithPath: show),
            output: URL(fileURLWithPath: out, isDirectory: true)
        )
    }

    func run(model: AppModel) {
        NSApp.appearance = NSAppearance(named: .darkAqua)
        try? FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        Task { @MainActor in
            await wait(seconds: 2)
            var waited = 0
            while model.isLoadingShow, waited < 60 {
                await wait(seconds: 0.5)
                waited += 1
            }
            mainWindow()?.setContentSize(NSSize(width: 1280, height: 760))
            mainWindow()?.center()
            await wait(seconds: 1)

            // Something playing, so pads show progress.
            let assigned = model.show.cues.map(\.slot).sorted()
            for slot in assigned.prefix(3) {
                model.trigger(slot)
            }
            await wait(seconds: 1.2)
            capture(mainWindow(), "1-pads")

            if !model.show.rundown.isEmpty {
                let wasShowing = model.showingRundown
                model.showingRundown = true
                // Standby on the second step, as if the first was fired.
                model.setStandby(model.show.rundown.firstIndex { $0.pad != nil })
                model.stepStandby(forward: true)
                mainWindow()?.setContentSize(NSSize(width: 1480, height: 760))
                await wait(seconds: 1)
                capture(mainWindow(), "9-rundown")
                model.showingLibrary = true
                await wait(seconds: 1)
                capture(mainWindow(), "10-library")
                model.showingLibrary = false
                model.showingRundown = wasShowing
                mainWindow()?.setContentSize(NSSize(width: 1280, height: 760))
                await wait(seconds: 0.5)
            }

            if let edited = model.show.cues.first(where: { !$0.markers.isEmpty }) ?? model.show.cues.first {
                model.editing = EditTarget(id: edited.slot)
                await wait(seconds: 3)  // waveforms load in the background
                capture(sheetWindow(), "2-pad-editor")
                model.editing = nil
                await wait(seconds: 1)
            }

            model.showingOutputs = true
            await wait(seconds: 1.5)
            capture(sheetWindow(), "3-outputs")
            model.showingOutputs = false
            await wait(seconds: 1)

            model.runPreflight()
            model.showingPreflight = true
            await wait(seconds: 1.5)
            capture(sheetWindow(), "4-pregame-check")
            model.showingPreflight = false
            await wait(seconds: 1)

            let remoteWasOn = model.remoteEnabled
            model.remoteEnabled = true
            model.showingControls = true
            await wait(seconds: 1.5)
            capture(sheetWindow(), "5-control")
            model.showingControls = false
            model.remoteEnabled = remoteWasOn
            await wait(seconds: 1)

            model.setGame("vs. Tigers")
            model.showingLog = true
            await wait(seconds: 1.5)
            capture(sheetWindow(), "6-game-log")
            model.showingLog = false
            await wait(seconds: 0.5)

            model.stopAll()
            NSApp.terminate(nil)
        }
    }

    private func wait(seconds: Double) async {
        try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    }

    private func mainWindow() -> NSWindow? {
        NSApp.windows.first { $0.isVisible && $0.sheetParent == nil && $0.contentView != nil && $0.frame.width > 400 }
    }

    private func sheetWindow() -> NSWindow? {
        mainWindow()?.attachedSheet ?? mainWindow()
    }

    /// Captures a window: first with the system `screencapture` tool (true
    /// rendering, including the title bar), falling back to drawing the
    /// window's views into a bitmap.
    private func capture(_ window: NSWindow?, _ name: String) {
        guard let window else { return }
        let url = output.appendingPathComponent("\(name).png")
        let tool = Process()
        tool.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        tool.arguments = ["-x", "-o", "-l", String(window.windowNumber), url.path]
        try? tool.run()
        tool.waitUntilExit()
        if tool.terminationStatus == 0,
           let size = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? Int,
           size > 10_000 {
            return
        }
        guard let view = window.contentView?.superview ?? window.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds)
        else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: url)
    }
}
