import Foundation
import AppKit
import ApplicationServices
import Darwin

private let pasteMarkerPrefix = "dev.caldwell.parakeet.paste."
private let restoredMarker = Data("restored".utf8)
private struct ClipboardDatum: Codable { let type: String; let data: Data }
private struct ClipboardJournal: Codable {
    let marker: String
    let board: String
    let original: [ClipboardDatum]
    var generations: [Int]
}

/// Single-item, eagerly materialized clipboard snapshots only. Unsupported or
/// oversized/promise data is refused before mutation. No clipboard data is logged.
final class ClipboardTransaction {
    private let board: NSPasteboard
    private let file: URL
    private var journal: ClipboardJournal
    private var finished = false
    private var marker: NSPasteboard.PasteboardType { NSPasteboard.PasteboardType(journal.marker) }
    init(board: NSPasteboard, directory: URL) throws {
        let generation = board.changeCount
        guard (board.pasteboardItems?.count ?? 0) <= 1 else { throw failure("Clipboard fallback cannot safely snapshot multiple clipboard items; text retained") }
        var original: [ClipboardDatum] = []; var size = 0
        for type in board.types ?? [] {
            if type.rawValue.hasPrefix(pasteMarkerPrefix) {
                guard board.data(forType: type) == restoredMarker else { throw failure("An interrupted Parakeet clipboard transaction needs recovery first") }
                continue
            }
            guard original.count < 64, let data = board.data(forType: type) else { throw failure("Clipboard contains unsupported promised data; text retained") }
            size += data.count
            guard size <= 16 * 1024 * 1024 else { throw failure("Clipboard snapshot exceeds 16 MiB; text retained") }
            original.append(ClipboardDatum(type: type.rawValue, data: data))
        }
        guard board.changeCount == generation else { throw failure("Clipboard changed while preparing insertion; text retained") }
        self.board = board
        let token = UUID().uuidString.lowercased()
        self.file = directory.appendingPathComponent(token + ".json")
        self.journal = ClipboardJournal(marker: pasteMarkerPrefix + token, board: board.name.rawValue, original: original, generations: [generation + 1])
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
        try save()
        guard board.changeCount == generation else { try? FileManager.default.removeItem(at: file); throw failure("Clipboard changed before insertion; text retained") }
    }
    private init(board: NSPasteboard, file: URL, journal: ClipboardJournal) { self.board = board; self.file = file; self.journal = journal }
    private func save() throws {
        try JSONEncoder().encode(journal).write(to: file, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        let handle = try FileHandle(forWritingTo: file); handle.synchronizeFile(); handle.closeFile()
    }
    private func ownsClipboard() -> Bool { (board.types ?? []).contains(marker) && journal.generations.contains(board.changeCount) }
    func begin(_ text: String) throws {
        // declareTypes is one generation change and registers the unique marker
        // even before its data is populated. A crash here remains recoverable.
        let count = board.declareTypes([.string, marker], owner: nil)
        guard journal.generations.contains(count), board.setString("pending", forType: marker), board.setString(text, forType: .string), ownsClipboard() else { throw failure("Clipboard ownership changed before paste; inspect target before retrying") }
    }
    fileprivate func prepareRestore() throws {
        guard ownsClipboard() else { throw failure("Clipboard changed elsewhere") }
        let previous = board.changeCount
        journal.generations = [previous, previous + 1]
        try save()
        guard board.changeCount == previous, ownsClipboard() else { throw failure("Clipboard changed elsewhere") }
        board.declareTypes(journal.original.map { NSPasteboard.PasteboardType($0.type) } + [marker], owner: nil)
    }
    @discardableResult func restore() throws -> String {
        if finished { return "already_completed" }
        if board.data(forType: marker) == restoredMarker {
            try FileManager.default.removeItem(at: file); finished = true; return "restored"
        }
        guard ownsClipboard() else {
            try FileManager.default.removeItem(at: file); finished = true; return "changed_elsewhere"
        }
        try prepareRestore()
        for datum in journal.original {
            guard ownsClipboard(), board.setData(datum.data, forType: NSPasteboard.PasteboardType(datum.type)) else { throw failure("Clipboard restoration interrupted; recovery journal retained") }
        }
        guard ownsClipboard(), board.setData(restoredMarker, forType: marker) else { throw failure("Clipboard restoration interrupted; recovery journal retained") }
        // The inert marker stays alongside the original types so restoration is
        // crash-safe through its final write. Future snapshots exclude it.
        try FileManager.default.removeItem(at: file); finished = true; return "restored"
    }
    static func recover(directory: URL, board: NSPasteboard) throws -> [String: Int] {
        guard FileManager.default.fileExists(atPath: directory.path) else { return ["restored": 0, "discarded": 0] }
        var restored = 0; var discarded = 0
        for file in try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.fileSizeKey]) where file.pathExtension == "json" && UUID(uuidString: file.deletingPathExtension().lastPathComponent) != nil {
            guard (try file.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? Int.max) <= 32 * 1024 * 1024 else { throw failure("Clipboard recovery journal exceeds its bound") }
            let journal = try JSONDecoder().decode(ClipboardJournal.self, from: Data(contentsOf: file))
            guard journal.board == board.name.rawValue, journal.marker == pasteMarkerPrefix + file.deletingPathExtension().lastPathComponent else { continue }
            let transaction = ClipboardTransaction(board: board, file: file, journal: journal)
            if try transaction.restore() == "restored" { restored += 1 } else { discarded += 1 }
        }
        return ["restored": restored, "discarded": discarded]
    }
}

func clipboardLease() throws -> Int32 {
    let path = NSTemporaryDirectory() + "dev.caldwell.parakeet-clipboard-\(getuid()).lock"
    let fd = open(path, O_CREAT | O_RDWR | O_CLOEXEC, S_IRUSR | S_IWUSR)
    guard fd >= 0 else { throw failure("Cannot open clipboard ownership lock") }
    guard flock(fd, LOCK_EX | LOCK_NB) == 0 else { close(fd); throw failure("Another Parakeet paste/recovery is active; text retained") }
    return fd
}
func recoverClipboard(_ directory: URL) throws -> [String: Int] {
    let lease = try clipboardLease(); defer { flock(lease, LOCK_UN); close(lease) }
    return try ClipboardTransaction.recover(directory: directory, board: .general)
}
private func textValue(_ element: AXUIElement) -> String? {
    var value: CFTypeRef?; guard AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &value) == .success else { return nil }; return value as? String
}
private func selection(_ element: AXUIElement) -> CFRange? {
    var value: CFTypeRef?; guard AXUIElementCopyAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, &value) == .success, let value = value, CFGetTypeID(value) == AXValueGetTypeID() else { return nil }
    var range = CFRange(); return AXValueGetValue(value as! AXValue, .cfRange, &range) ? range : nil
}
func clipboardInsert(element: AXUIElement, pid: pid_t, token: String, text: String, directory: URL) throws -> [String: Any] {
    let lease = try clipboardLease(); defer { flock(lease, LOCK_UN); close(lease) }
    _ = try ClipboardTransaction.recover(directory: directory, board: .general)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, String(CFHash(try focused(pid))) == token, let before = textValue(element) else { throw failure("Clipboard paste requires the original app/element in front and a readable value; text retained") }
    let range = selection(element) ?? (before.isEmpty ? CFRange(location: 0, length: 0) : nil)
    guard let range = range, range.location >= 0, range.length >= 0, range.location <= (before as NSString).length, range.length <= (before as NSString).length - range.location else { throw failure("Cannot verify the target selection for clipboard fallback; text retained") }
    let expected = NSMutableString(string: before); expected.replaceCharacters(in: NSRange(location: range.location, length: range.length), with: text)
    let transaction = try ClipboardTransaction(board: .general, directory: directory)
    var verified = false
    var operationError: Error?
    do {
        try transaction.begin(text)
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid, String(CFHash(try focused(pid))) == token, textValue(element) == before else { throw failure("Original target changed before paste; inspect before retrying") }
        let current = selection(element) ?? (before.isEmpty ? CFRange(location: 0, length: 0) : nil)
        guard let current = current, current.location == range.location, current.length == range.length else { throw failure("Original selection changed or became unreadable before paste; text retained") }
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 9, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: 9, keyDown: false) else { throw failure("Cannot create paste events; text retained") }
        down.flags = .maskCommand; up.flags = .maskCommand; down.postToPid(pid); up.postToPid(pid)
        let deadline = Date().addingTimeInterval(1)
        repeat {
            if String(CFHash(try focused(pid))) != token { break }
            if textValue(element) == (expected as String) { verified = true; break }
            usleep(20_000)
        } while Date() < deadline
    } catch { operationError = error }
    let restoration: String
    do { restoration = try transaction.restore() }
    catch { throw failure("Paste may have occurred. Clipboard restoration failed; private recovery journal retained. Inspect the target and use Recover interrupted clipboard paste before retrying") }
    if let error = operationError { throw error }
    guard verified else { throw failure("Paste could not be verified; text retained. Inspect the original app before retrying to avoid duplication") }
    return ["inserted": true, "method": "clipboard", "verification": "value_readback", "clipboard_status": restoration]
}

func clipboardSelfTest() throws -> [String: Any] {
    let board = NSPasteboard.withUniqueName(); defer { board.releaseGlobally() }
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("parakeet-clipboard-test-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let custom = NSPasteboard.PasteboardType("dev.parakeet.test-binary")
    board.declareTypes([.string, custom], owner: nil); board.setString("original fixture", forType: .string); board.setData(Data([1, 2, 3]), forType: custom)
    let transaction = try ClipboardTransaction(board: board, directory: directory); try transaction.begin("temporary"); _ = try transaction.restore()
    guard board.string(forType: .string) == "original fixture", board.data(forType: custom) == Data([1, 2, 3]) else { throw failure("Typed clipboard restoration failed") }
    let external = try ClipboardTransaction(board: board, directory: directory); try external.begin("temporary"); board.clearContents(); board.setString("new user copy", forType: .string)
    guard try external.restore() == "changed_elsewhere", board.string(forType: .string) == "new user copy" else { throw failure("New clipboard generation overwritten") }
    let interrupted = try ClipboardTransaction(board: board, directory: directory); try interrupted.begin("temporary"); try interrupted.prepareRestore()
    _ = try ClipboardTransaction.recover(directory: directory, board: board)
    guard board.string(forType: .string) == "new user copy" else { throw failure("Interrupted restoration recovery failed") }
    let child = Process(); child.executableURL = URL(fileURLWithPath: CommandLine.arguments[0]); child.arguments = ["clipboard-test-child", board.name.rawValue, directory.path]; let pipe = Pipe(); child.standardOutput = pipe; child.standardError = FileHandle.nullDevice
    try child.run(); defer { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
    let ready = pipe.fileHandleForReading.availableData
    guard String(data: ready, encoding: .utf8)?.contains("clipboard_test_ready") == true else { throw failure("Clipboard test child did not become ready") }
    let journals=try FileManager.default.contentsOfDirectory(at:directory,includingPropertiesForKeys:nil).filter{$0.pathExtension=="json"}
    guard journals.count==1,(try FileManager.default.attributesOfItem(atPath:journals[0].path)[.posixPermissions] as? NSNumber)?.intValue==0o600 else {throw failure("Clipboard journal permissions incorrect")}
    kill(child.processIdentifier, SIGKILL); child.waitUntilExit()
    let recovered = try ClipboardTransaction.recover(directory: directory, board: board)
    guard recovered["restored"] == 1, board.string(forType: .string) == "new user copy" else { throw failure("Process-death clipboard recovery failed") }
    let attributes = try FileManager.default.attributesOfItem(atPath: directory.path)
    guard (attributes[.posixPermissions] as? NSNumber)?.intValue == 0o700 else { throw failure("Clipboard directory permissions incorrect") }
    let first=NSPasteboardItem();first.setString("a",forType:.string);let second=NSPasteboardItem();second.setString("b",forType:.string);board.clearContents();board.writeObjects([first,second]);let before=board.changeCount
    var refused=false;do{_ = try ClipboardTransaction(board:board,directory:directory)}catch{refused=true}
    guard refused,board.changeCount==before,board.pasteboardItems?.count==2 else{throw failure("Multiple clipboard items were not preserved")}
    return ["multiple_items_refused_without_mutation":true,"private_journal_permissions":true,"private_pasteboard_only": true, "typed_data_restored": true, "new_copy_preserved": true, "mid_restore_recovered": true, "sigkill_recovered": true, "private_directory_permissions": true, "general_clipboard_touched": false]
}
func clipboardTestChild(board: String, directory: String) throws {
    guard board != NSPasteboard.general.name.rawValue else {throw failure("Clipboard test child requires a private pasteboard")}
    let transaction = try ClipboardTransaction(board: NSPasteboard(name: NSPasteboard.Name(board)), directory: URL(fileURLWithPath: directory)); try transaction.begin("temporary child fixture")
    emit(["clipboard_test_ready": true]); while true { sleep(1) }
}
