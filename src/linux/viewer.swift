import AppKit
import WebKit

final class LinuxViewer: NSObject, NSApplicationDelegate, NSWindowDelegate, WKNavigationDelegate {
    private var window: NSWindow?
    private var web: WKWebView?
    private let parent = getppid()

    func applicationDidFinishLaunching(_ notification: Notification) {
        let environment = ProcessInfo.processInfo.environment
        guard let address = environment["PISTON_VIEWER_URL"], let url = URL(string: address) else {
            NSApp.terminate(nil)
            return
        }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 780),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable],
                              backing: .buffered, defer: false)
        window.title = "Piston Linux — " + (environment["PISTON_VIEWER_TITLE"] ?? "Application")
        window.isReleasedWhenClosed = false
        window.delegate = self
        let web = WKWebView(frame: window.contentView!.bounds)
        web.autoresizingMask = [.width, .height]
        web.navigationDelegate = self
        window.contentView = web
        self.window = window
        self.web = web
        let menu = NSMenu()
        let app = NSMenuItem()
        menu.addItem(app)
        let commands = NSMenu()
        commands.addItem(withTitle: "Close Linux VM", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        commands.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        app.submenu = commands
        NSApp.mainMenu = menu
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        web.load(URLRequest(url: url))
        Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            guard let self else { return }
            if getppid() != self.parent { NSApp.terminate(nil) }
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        decisionHandler(navigationAction.request.url?.host == "127.0.0.1" ? .allow : .cancel)
    }
}

let application = NSApplication.shared
let delegate = LinuxViewer()
application.delegate = delegate
application.setActivationPolicy(.regular)
application.run()
