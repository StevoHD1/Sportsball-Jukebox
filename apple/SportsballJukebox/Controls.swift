import AppKit
import SwiftUI

/// A thin bar between two panels; drag it to resize the panel beside it.
struct ResizeHandle: View {
    @Binding var width: Double
    let range: ClosedRange<Double>
    /// 1 when the panel is left of the handle (dragging right widens it),
    /// -1 when it is on the right.
    let direction: Double
    @State private var startWidth: Double?
    @State private var hovering = false

    var body: some View {
        Rectangle()
            .fill(Color.secondary.opacity(hovering || startWidth != nil ? 0.5 : 0.2))
            .frame(width: 1)
            .padding(.horizontal, 3)
            .contentShape(Rectangle())
            .onHover { inside in
                hovering = inside
                if inside {
                    NSCursor.resizeLeftRight.push()
                } else {
                    NSCursor.pop()
                }
            }
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { value in
                        let base = startWidth ?? width
                        startWidth = base
                        let proposed = base + direction * Double(value.translation.width)
                        width = min(max(proposed, range.lowerBound), range.upperBound)
                    }
                    .onEnded { _ in startWidth = nil }
            )
            .help("Drag to resize")
            .accessibilityHidden(true)
    }
}

/// A progress bar you can click or drag to jump to a point.
struct ScrubBar: View {
    /// 0...1.
    let progress: Double
    /// Seconds the whole bar stands for.
    let duration: Double
    var tint: Color = .green
    let onSeek: (Double) -> Void
    @State private var dragging: Double?

    var body: some View {
        GeometryReader { geometry in
            let width = max(geometry.size.width, 1)
            let shown = min(max(dragging ?? progress, 0), 1)
            ZStack(alignment: .leading) {
                Capsule().fill(Color.secondary.opacity(0.25))
                Capsule().fill(tint).frame(width: shown * width)
                if dragging != nil {
                    Circle()
                        .fill(Color.white)
                        .frame(width: 10, height: 10)
                        .offset(x: shown * width - 5)
                }
            }
            .frame(height: 6)
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        dragging = min(max(Double(value.location.x / width), 0), 1)
                    }
                    .onEnded { value in
                        let fraction = min(max(Double(value.location.x / width), 0), 1)
                        dragging = nil
                        if duration > 0 { onSeek(fraction * duration) }
                    }
            )
        }
        .frame(height: 16)
        .help("Click or drag to jump to a point")
        .accessibilityElement()
        .accessibilityLabel("Position")
        .accessibilityValue("\(formatTime(progress * duration)) of \(formatTime(duration))")
        .accessibilityAdjustableAction { direction in
            let step = direction == .increment ? 5.0 : -5.0
            onSeek(min(max(progress * duration + step, 0), duration))
        }
    }
}
