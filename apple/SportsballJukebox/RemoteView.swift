import CoreImage
import CoreImage.CIFilterBuiltins
import Darwin
import SwiftUI

/// Settings for the phone/tablet remote: on/off, port, PIN, and the
/// addresses (with a QR code) to open on the phone.
struct RemoteSection: View {
    @EnvironmentObject private var model: AppModel
    @State private var portText = ""
    @State private var addresses: [String] = []

    var body: some View {
        GroupBox("Phone & tablet remote") {
            HStack(alignment: .top, spacing: 16) {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Toggle("Serve the remote on TCP port", isOn: $model.remoteEnabled)
                        TextField("8765", text: $portText)
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 70)
                            .onSubmit(applyPort)
                        Button("Apply", action: applyPort)
                            .disabled(portText == String(model.remotePort))
                    }
                    HStack(spacing: 8) {
                        Text("PIN")
                        Text(model.remotePin)
                            .font(.system(.title3, design: .monospaced).weight(.semibold))
                            .textSelection(.enabled)
                        Button("New PIN") { model.regenerateRemotePin() }
                            .help("Phones using the old PIN will be asked for the new one")
                    }
                    if model.status?.remotePort != nil, let first = urls.first {
                        Label("Running. On a phone on the same Wi-Fi, scan the code or open:", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        ForEach(urls, id: \.self) { url in
                            Text(url)
                                .font(.system(.callout, design: .monospaced))
                                .textSelection(.enabled)
                        }
                        .accessibilityHint("Open this address on a phone or tablet")
                        if urls.count > 1 {
                            Text("The code opens \(first).")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    } else if model.status?.remotePort != nil {
                        Text("Running, but this Mac has no network address. Connect it to Wi-Fi or Ethernet.")
                            .foregroundStyle(.orange)
                    } else {
                        Text("Fire pads, switch banks and stop everything from any phone or tablet browser on the same network. No app to install.")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 0)
                if model.status?.remotePort != nil, let first = urls.first {
                    QRCodeView(text: first + "#pin=" + model.remotePin)
                        .frame(width: 132, height: 132)
                        .accessibilityLabel("QR code for the remote page")
                }
            }
            .padding(6)
        }
        .onAppear {
            portText = String(model.remotePort)
            addresses = Self.localAddresses()
        }
        .onChange(of: model.status?.remotePort) { _, _ in
            addresses = Self.localAddresses()
        }
    }

    private var urls: [String] {
        let port = model.status?.remotePort ?? model.remotePort
        return addresses.map { "http://\($0):\(port)/" }
    }

    private func applyPort() {
        let port = Int(portText.trimmingCharacters(in: .whitespaces)) ?? 0
        model.remotePort = min(max(port, 1024), 65535)
        portText = String(model.remotePort)
    }

    /// This Mac's IPv4 addresses on Wi-Fi/Ethernet (not loopback or
    /// self-assigned), en0 first.
    static func localAddresses() -> [String] {
        var found: [(name: String, address: String)] = []
        var list: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&list) == 0, let first = list else { return [] }
        defer { freeifaddrs(list) }
        var cursor: UnsafeMutablePointer<ifaddrs>? = first
        while let entry = cursor {
            defer { cursor = entry.pointee.ifa_next }
            let flags = Int32(entry.pointee.ifa_flags)
            guard (flags & IFF_UP) != 0, (flags & IFF_LOOPBACK) == 0,
                  let addr = entry.pointee.ifa_addr,
                  addr.pointee.sa_family == UInt8(AF_INET)
            else { continue }
            var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            let result = getnameinfo(
                addr, socklen_t(addr.pointee.sa_len), &host, socklen_t(host.count),
                nil, 0, NI_NUMERICHOST
            )
            guard result == 0 else { continue }
            let address = String(cString: host)
            if address.hasPrefix("169.254.") { continue }
            found.append((String(cString: entry.pointee.ifa_name), address))
        }
        return found
            .sorted { ($0.name == "en0" ? 0 : 1, $0.name) < ($1.name == "en0" ? 0 : 1, $1.name) }
            .map(\.address)
    }
}

/// A crisp QR code for `text`.
struct QRCodeView: View {
    let text: String

    var body: some View {
        if let image = Self.render(text) {
            Image(decorative: image, scale: 1)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .padding(6)
                .background(Color.white, in: RoundedRectangle(cornerRadius: 8))
        }
    }

    private static let context = CIContext()

    static func render(_ text: String) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        return context.createCGImage(output, from: output.extent)
    }
}
