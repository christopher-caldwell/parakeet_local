import Foundation
struct AppLogger {
    init(category:String) {}
    func info(_ message:String) { }
    func warning(_ message:String) { FileHandle.standardError.write(Data((message+"\n").utf8)) }
    func debug(_ message:String) { }
    func error(_ message:String) { FileHandle.standardError.write(Data((message+"\n").utf8)) }
}
