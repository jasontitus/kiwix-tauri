import ObjectiveC
import UIKit
import WebKit

private var shortcutRelayKey: UInt8 = 0

// The selector participates in the existing responder chain, including WebKit's
// cross-origin content responders. It neither replaces nor swizzles a responder.
// Only command objects registered by our router carry an authorized relay.
private extension UIResponder {
  @objc func kiwixDispatchHardwareShortcut(_ command: UIKeyCommand) {
    (objc_getAssociatedObject(command, &shortcutRelayKey) as? ShortcutRelay)?.dispatch(command)
  }
}

private final class ShortcutRelay {
  weak var router: HardwareShortcutRouter?
  let action: String
  init(router: HardwareShortcutRouter, action: String) { self.router = router; self.action = action }
  func dispatch(_ command: UIKeyCommand) { router?.dispatch(command, action: action) }
}

final class HardwareShortcutRouter {
  private(set) weak var hostController: UIViewController?
  private(set) weak var hostScene: UIWindowScene?
  private weak var webView: WKWebView?
  private var commands: [UIKeyCommand] = []
  private var actions: [ObjectIdentifier: String] = [:]
  private let emit: (String) -> Void

  init(emit: @escaping (String) -> Void) { self.emit = emit }

  func bind(controller: UIViewController?, webView: WKWebView?) {
    guard let controller = controller, let webView = webView else { unbind(); return }
    if hostController === controller, self.webView === webView, !commands.isEmpty {
      hostScene = controller.viewIfLoaded?.window?.windowScene
      return
    }
    unbind()
    hostController = controller
    self.webView = webView
    hostScene = controller.viewIfLoaded?.window?.windowScene
    let modified: [(String, String)] = [
      ("o", "import"), ("t", "new-tab"), ("w", "close-tab"),
      ("k", "search"), ("f", "find"), ("p", "print"),
      ("s", "save"), ("d", "bookmark"), ("=", "zoom-in"),
      ("+", "zoom-in"), ("-", "zoom-out"), ("0", "zoom-reset")
    ]
    let modifiers: [UIKeyModifierFlags] = [.command, .control]
    for modifier in modifiers {
      for (input, action) in modified { add(input, modifier, action: action, to: controller) }
      add("t", [modifier, .shift], action: "reopen-tab", to: controller)
      // Layouts producing '+' as Shift+'=' need the physical combination too.
      add("=", [modifier, .shift], action: "zoom-in", to: controller)
      add("+", [modifier, .shift], action: "zoom-in", to: controller)
    }
    add("\t", .control, action: "next-tab", to: controller)
    add("\t", [.control, .shift], action: "previous-tab", to: controller)
    for digit in 0...9 { add(String(digit), .alternate, action: "select-tab-" + String(digit), to: controller) }
    add(UIKeyCommand.inputLeftArrow, .alternate, action: "back", to: controller)
    add(UIKeyCommand.inputRightArrow, .alternate, action: "forward", to: controller)
    add(UIKeyCommand.f11, [], action: "fullscreen", to: controller)
    add(UIKeyCommand.inputEscape, [], action: "escape", to: controller)
  }

  private func add(_ input: String, _ modifiers: UIKeyModifierFlags, action: String, to controller: UIViewController) {
    let command = UIKeyCommand(input: input, modifierFlags: modifiers,
                               action: #selector(UIResponder.kiwixDispatchHardwareShortcut(_:)))
    // No hard-coded discoverability strings. System layout localization remains
    // enabled; ordinary system/text-editing shortcuts keep their precedence.
    if #available(iOS 15.0, *) { command.wantsPriorityOverSystemBehavior = false }
    let relay = ShortcutRelay(router: self, action: action)
    objc_setAssociatedObject(command, &shortcutRelayKey, relay, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    commands.append(command)
    actions[ObjectIdentifier(command)] = action
    controller.addKeyCommand(command)
  }

  fileprivate func dispatch(_ command: UIKeyCommand, action: String) {
    guard Thread.isMainThread, actions[ObjectIdentifier(command)] == action,
          UIApplication.shared.applicationState == .active,
          let controller = hostController,
          let scene = controller.viewIfLoaded?.window?.windowScene,
          scene.activationState == .foregroundActive,
          webView?.window?.windowScene === scene,
          controller.presentedViewController == nil else { return }
    // UIKit consumes a matched key command; no DOM event or archive message is
    // used to manufacture application actions or their native-origin marker.
    emit(action)
  }

  func unbind() {
    for command in commands {
      hostController?.removeKeyCommand(command)
      objc_setAssociatedObject(command, &shortcutRelayKey, nil, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    }
    commands.removeAll()
    actions.removeAll()
    hostController = nil
    webView = nil
    hostScene = nil
  }

  deinit {
    // Plugin teardown may occur off-main. Detach associated relays immediately;
    // remove only our own commands from the old actual controller on main.
    let oldCommands = commands
    let controller = hostController
    for command in oldCommands { objc_setAssociatedObject(command, &shortcutRelayKey, nil, .OBJC_ASSOCIATION_RETAIN_NONATOMIC) }
    if Thread.isMainThread { oldCommands.forEach { controller?.removeKeyCommand($0) } }
    else { DispatchQueue.main.async { oldCommands.forEach { controller?.removeKeyCommand($0) } } }
  }
}
