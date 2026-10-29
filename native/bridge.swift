import Foundation
import AVFoundation
import AudioToolbox
import CoreAudio
import ScreenCaptureKit
import CoreMedia
import AppKit
import ApplicationServices

func emit(_ object: [String: Any]) {
    if let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]), let string = String(data: data, encoding: .utf8) { print(string); fflush(stdout) }
}
func failure(_ message: String) -> NSError { NSError(domain: "ParakeetLocal", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
func monotonic() -> Double { AVAudioTime.seconds(forHostTime: mach_absolute_time()) }
func stringProperty(_ device: AudioDeviceID, _ selector: AudioObjectPropertySelector) -> String {
    var address = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var result: CFString = "" as CFString; var size = UInt32(MemoryLayout<CFString>.size)
    guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &result) == noErr else { return "" }
    return result as String
}
func aggregateComposition(_ device: AudioDeviceID) -> [String: Any]? {
    var address = AudioObjectPropertyAddress(mSelector: kAudioAggregateDevicePropertyComposition, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    guard AudioObjectHasProperty(device, &address) else { return nil }
    var result: Unmanaged<CFDictionary>?
    var size = UInt32(MemoryLayout<Unmanaged<CFDictionary>?>.size)
    guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &result) == noErr,
          let result = result else { return nil }
    return result.takeRetainedValue() as? [String: Any]
}
func activeSubDeviceUIDs(_ device: AudioDeviceID) -> [String] {
    var address = AudioObjectPropertyAddress(mSelector: kAudioAggregateDevicePropertyActiveSubDeviceList, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(device, &address, 0, nil, &size) == noErr, size > 0 else { return [] }
    var devices = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &devices) == noErr else { return [] }
    return devices.map { stringProperty($0, kAudioDevicePropertyDeviceUID) }
}
/// An output-only member is harmless; another input, an inactive input, or missing
/// channel metadata is not proof that the selected microphone owns the samples.
func aggregateUsesSelectedInput(_ composition: [String: Any], activeUIDs: [String], selectedUID: String) -> Bool {
    guard let members = composition[kAudioAggregateDeviceSubDeviceListKey] as? [[String: Any]], !members.isEmpty else { return false }
    var inputs: [String] = []
    for member in members {
        guard let channels = member[kAudioSubDeviceInputChannelsKey] as? Int, channels >= 0 else { return false }
        if channels > 0 {
            guard let uid = member[kAudioSubDeviceUIDKey] as? String else { return false }
            inputs.append(uid)
        }
    }
    return inputs == [selectedUID] && activeUIDs.contains(selectedUID)
}
func matchesSelectedInput(_ device: AudioDeviceID, _ uid: String) -> Bool {
    if stringProperty(device, kAudioDevicePropertyDeviceUID) == uid { return true }
    guard let composition = aggregateComposition(device) else { return false }
    return aggregateUsesSelectedInput(composition, activeUIDs: activeSubDeviceUIDs(device), selectedUID: uid)
}
func inputDevices() throws -> [[String: Any]] {
    var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr else { throw failure("Cannot enumerate Core Audio devices") }
    var ids = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
    guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &ids) == noErr else { throw failure("Cannot read Core Audio devices") }
    var def = AudioDeviceID(0); var defSize = UInt32(MemoryLayout<AudioDeviceID>.size)
    var da = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultInputDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &da, 0, nil, &defSize, &def)
    var result: [[String: Any]] = []
    for id in ids {
        var a = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreams, mScope: kAudioDevicePropertyScopeInput, mElement: kAudioObjectPropertyElementMain); var count: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(id, &a, 0, nil, &count) == noErr, count > 0 else { continue }
        result.append(["device_id": id, "uid": stringProperty(id, kAudioDevicePropertyDeviceUID), "name": stringProperty(id, kAudioObjectPropertyName), "is_default": id == def])
    }
    return result
}
func focused(_ pid: pid_t) throws -> AXUIElement {
    let app = AXUIElementCreateApplication(pid); var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(app, kAXFocusedUIElementAttribute as CFString, &value) == .success, let value = value, CFGetTypeID(value) == AXUIElementGetTypeID() else { throw failure("Focused accessibility element unavailable") }
    return unsafeBitCast(value, to: AXUIElement.self)
}
func target(bundle requestedBundle:String? = nil) throws -> [String: Any] {
    guard AXIsProcessTrusted() else { throw failure("Accessibility permission is required; no prompt was requested") }
    let application:NSRunningApplication?
    if let requestedBundle=requestedBundle {application=NSWorkspace.shared.runningApplications.first(where:{$0.bundleIdentifier==requestedBundle})}else{application=NSWorkspace.shared.frontmostApplication}
    guard let app=application,let bundle=app.bundleIdentifier else {throw failure("No identifiable target application") }
    let element = try focused(app.processIdentifier)
    return ["bundle_id": bundle, "process_id": app.processIdentifier, "element_token": String(CFHash(element))]
}
func insert(_ options: [String: Any]) throws -> [String:Any] {
    guard AXIsProcessTrusted(), let pid = options["process_id"] as? Int32, let bundle = options["bundle_id"] as? String, let token = options["element_token"] as? String, let text = options["text"] as? String else { throw failure("Insertion needs accessibility permission, saved target, and text") }
    guard NSRunningApplication(processIdentifier: pid)?.bundleIdentifier == bundle else { throw failure("Original target process is no longer available") }
    let element = try focused(pid)
    guard String(CFHash(element)) == token else { throw failure("Original focused element changed; recognized text was retained") }
    if AXUIElementSetAttributeValue(element, kAXSelectedTextAttribute as CFString, text as CFString) == .success { return ["inserted":true,"method":"accessibility","verification":"accessibility_acknowledged"] }
    guard let directory=options["clipboard_recovery_directory"] as? String else {throw failure("Target does not support direct insertion; clipboard recovery is not configured. Text retained")}
    return try clipboardInsert(element:element,pid:pid,token:token,text:text,directory:URL(fileURLWithPath:directory))
}

/// Places each buffer on the source timeline from host timestamps without ever discarding audio.
struct Timeline {
    struct Step { var gapFrames: Int64 = 0; var driftFrames: Int64 = 0 }
    /// `first` is the host time of the first frame; it is re-based when the device clock runs ahead.
    func place(hostSeconds: Double, first: inout Double, rate: Double, timeline: Int64) -> Step {
        let expected = Int64(max(0, (hostSeconds - first) * rate))
        let delta = expected - timeline
        var step = Step()
        if delta > Int64(rate * 0.1) { step.gapFrames = delta }
        else if delta < -Int64(rate * 0.1) { step.driftFrames = -delta; first += Double(delta) / rate }
        return step
    }
}
final class TrackWriter {
    let source: String; let path: URL; let origin: Double
    private let queue: DispatchQueue; private let lock = NSLock(); private var pending = 0
    private var healthyFrames: Int64 = 0; private var lastCallback: Double = 0; private var latestPeak:Float = 0; private var sessionPeak:Float = 0; private var healthRate:Double = 0
    private var converter: AVAudioConverter?
    private var formatChanges: [[String: Any]] = []
    private var lastInputFormat: AVAudioFormat?
    private var lastInputEnd: Double?
    private var file: AVAudioFile?; private var first: Double?; private var rate: Double = 0
    private var written: Int64 = 0; private var timeline: Int64 = 0; private var dropped = 0; private var gaps: [[String: Any]] = []; private var errors: [String] = []; private var failed=false; private var peak: Float = 0; private var drift: [[String: Any]] = []; private var timeline_ = Timeline()
    init(source: String, directory: URL, origin: Double) { self.source = source; self.path = directory.appendingPathComponent(source + "-raw.caf"); self.origin = origin; self.queue = DispatchQueue(label: "parakeet.writer." + source) }
    // Keep a stable recording format through Bluetooth/device renegotiation. Conversion
    // happens on the writer queue, never on the real-time callback. The archive later
    // preserves these captured PCM samples exactly; it performs no further quantization.
    private func recordingBuffer(_ input: AVAudioPCMBuffer) throws -> AVAudioPCMBuffer {
        let target = file?.processingFormat ?? AVAudioFormat(standardFormatWithSampleRate: max(48000, input.format.sampleRate), channels: max(2, input.format.channelCount))!
        if lastInputFormat != input.format {
            formatChanges.append(["atFrame": timeline, "inputSampleRate": input.format.sampleRate, "inputChannels": input.format.channelCount, "recordingSampleRate": target.sampleRate, "recordingChannels": target.channelCount])
            lastInputFormat = input.format
            converter = nil
        }
        if input.format == target { return input }
        if converter == nil {
            converter = AVAudioConverter(from: input.format, to: target)
            converter?.primeMethod = .normal
        }
        guard let converter = converter,
              let output = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: AVAudioFrameCount(ceil(Double(input.frameLength) * target.sampleRate / input.format.sampleRate)) + 32) else {
            throw failure("Cannot adapt the microphone's new audio format")
        }
        var supplied = false
        var error: NSError?
        let status = converter.convert(to: output, error: &error) { _, state in
            if supplied { state.pointee = .noDataNow; return nil }
            supplied = true; state.pointee = .haveData; return input
        }
        if status == .error { throw error ?? failure("Microphone format conversion failed") }
        return output
    }
    // Only actual input callbacks establish liveness. Draining delayed converter
    // samples must never make a stalled device look healthy again.
    func enqueue(_ input: AVAudioPCMBuffer, hostSeconds: Double) {
        lock.lock(); if pending >= 32 { dropped += 1; lock.unlock(); return }; pending += 1; lastCallback = monotonic(); lock.unlock()
        queue.async { [self] in
            defer { lock.lock(); pending -= 1; lock.unlock() }
            do {
                if failed{return}
                if lastInputFormat != input.format || lastInputEnd.map({ hostSeconds - $0 > 0.1 }) == true { try flushConverter() }
                lastInputEnd = hostSeconds + Double(input.frameLength) / input.format.sampleRate
                let buffer = try recordingBuffer(input)
                if buffer.frameLength == 0 { return }
                try writePCM(buffer, hostSeconds: hostSeconds)
            } catch { failed=true;errors.append(error.localizedDescription);emit(["event":"source_failed","source":source,"message":error.localizedDescription]) }
        }
    }
    private func writePCM(_ buffer: AVAudioPCMBuffer, hostSeconds: Double) throws {
                if file == nil { file = try AVAudioFile(forWriting: path, settings: buffer.format.settings, commonFormat: buffer.format.commonFormat, interleaved: buffer.format.isInterleaved); rate = buffer.format.sampleRate; first = hostSeconds; emit(["event": "first_buffer", "source": source, "start_offset_ms": max(0, Int((hostSeconds-origin)*1000)), "sample_rate": rate]) }
                guard let first = first, let file = file else { return }
                guard buffer.format.sampleRate==rate && buffer.format.channelCount==file.processingFormat.channelCount else{throw failure("Source audio format changed during capture; retained original format audio")}
                let step = timeline_.place(hostSeconds: hostSeconds, first: &self.first!, rate: rate, timeline: timeline)
                if step.gapFrames > 0 {
                    guard step.gapFrames < Int64(rate*7200) else { throw failure("Implausible source timestamp jump") }
                    gaps.append(["startFrame": timeline, "frames": step.gapFrames, "reason": "capture_timestamp_gap"])
                    var missing = step.gapFrames
                    while missing > 0 { let count = AVAudioFrameCount(min(missing, 4096)); guard let silence = AVAudioPCMBuffer(pcmFormat: buffer.format, frameCapacity: count) else { throw failure("Cannot allocate gap buffer") }; silence.frameLength = count; for audio in UnsafeMutableAudioBufferListPointer(silence.mutableAudioBufferList) { if let data = audio.mData { memset(data, 0, Int(audio.mDataByteSize)) } }; try file.write(from: silence); missing -= Int64(count) }
                    timeline += step.gapFrames
                }
                // A device clock that runs ahead of the host clock is recorded, never answered by
                // dropping audio: every captured frame is written and the timeline is re-based.
                if step.driftFrames > 0 { drift.append(["atFrame": timeline, "framesAhead": step.driftFrames]) }
                var bufferPeak:Float=0
                let count=Int(buffer.frameLength);let channels=Int(buffer.format.channelCount)
                for channel in 0..<channels { for frame in 0..<count {
                    let bank=buffer.format.isInterleaved ? 0:channel;let index=buffer.format.isInterleaved ? frame*channels+channel:frame
                    let sample:Float
                    if let data=buffer.floatChannelData {sample=data[bank][index]}else if let data=buffer.int16ChannelData {sample=Float(data[bank][index])/32768}else if let data=buffer.int32ChannelData {sample=Float(data[bank][index])/2147483648}else{sample=0}
                    if sample.isFinite{bufferPeak=max(bufferPeak,abs(sample))}
                }}
                peak=max(peak,bufferPeak)
                try file.write(from: buffer); written += Int64(buffer.frameLength); timeline += Int64(buffer.frameLength); lock.lock(); healthyFrames=written; latestPeak=bufferPeak;sessionPeak=max(sessionPeak,bufferPeak);healthRate=rate; lock.unlock()
    }
    private func flushConverter() throws {
        guard let converter = converter, let file = file else { return }
        while true {
            let output = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 4096)!
            var error: NSError?
            let status = converter.convert(to: output, error: &error) { _, state in state.pointee = .endOfStream; return nil }
            if status == .error { throw error ?? failure("Cannot finish microphone format conversion") }
            if output.frameLength > 0 { try writePCM(output, hostSeconds: (first ?? origin) + Double(timeline) / rate) }
            if status == .endOfStream || output.frameLength == 0 { break }
        }
        self.converter = nil
    }
    func markFailure(_ reason:String){queue.async{[self] in failed=true;errors.append(reason);emit(["event":"source_failed","source":source,"message":reason])}}
    func health() -> [String: Any] { lock.lock(); defer { lock.unlock() }; return ["peak_amplitude":latestPeak,"session_peak_amplitude":sessionPeak,"sample_rate":healthRate,"written_frames": healthyFrames, "seconds_since_buffer": lastCallback > 0 ? monotonic()-lastCallback : NSNull(), "first_buffer_ready": healthyFrames > 0, "is_receiving": healthyFrames > 0 && monotonic()-lastCallback < 3] }
    func report() -> [String: Any] {
        queue.sync {
            do { if !failed { try flushConverter() } }
            catch { errors.append(error.localizedDescription) }
            file = nil
        }; lock.lock(); let stalled=healthyFrames>0 && monotonic()-lastCallback>3; lock.unlock(); if stalled{errors.append("Source stalled before finalization")}
        lock.lock(); let drops = dropped; lock.unlock()
        return ["startOffsetMs": first.map { max(0, Int(($0-origin)*1000)) } as Any? ?? NSNull(), "sampleRate": rate, "writtenFrameCount": written, "timelineFrameCount": timeline, "droppedBuffers": drops, "gaps": gaps, "clockDrift": drift, "inputFormats": formatChanges, "errors": errors, "peakAmplitude": peak, "firstBufferReceived": first != nil]
    }
}
func copyBuffer(_ original: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
    guard let copy = AVAudioPCMBuffer(pcmFormat: original.format, frameCapacity: original.frameLength) else { return nil }; copy.frameLength = original.frameLength
    let dst = UnsafeMutableAudioBufferListPointer(copy.mutableAudioBufferList); let src = UnsafeMutableAudioBufferListPointer(original.mutableAudioBufferList)
    for i in 0..<min(dst.count, src.count) { if let d = dst[i].mData, let s = src[i].mData { memcpy(d,s,Int(min(dst[i].mDataByteSize,src[i].mDataByteSize))) } }; return copy
}
final class SystemOutput: NSObject, SCStreamOutput, SCStreamDelegate {
    let writer: TrackWriter; var errors: [String] = []
    init(_ writer: TrackWriter) { self.writer = writer }
    func stream(_ stream: SCStream, didOutputSampleBuffer sample: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, CMSampleBufferDataIsReady(sample), let description = CMSampleBufferGetFormatDescription(sample), let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(description), let format = AVAudioFormat(streamDescription: asbd), let pcm = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(CMSampleBufferGetNumSamples(sample))) else { return }
        pcm.frameLength = pcm.frameCapacity
        guard CMSampleBufferCopyPCMDataIntoAudioBufferList(sample, at: 0, frameCount: Int32(pcm.frameLength), into: pcm.mutableAudioBufferList) == noErr else { return }
        let timestamp = CMSampleBufferGetPresentationTimeStamp(sample).seconds
        writer.enqueue(pcm, hostSeconds: timestamp.isFinite ? timestamp : monotonic())
    }
    func stream(_ stream: SCStream, didStopWithError error: Error) { errors.append(error.localizedDescription); emit(["event":"source_failed","source":"system","message":error.localizedDescription]) }
}
final class StopFlag { let lock = NSLock(); private var stopped = false; func stop() { lock.lock(); stopped = true; lock.unlock() }; func get() -> Bool { lock.lock(); defer {lock.unlock()};return stopped } }
func startMicrophone(_ uid: String?, _ mic: TrackWriter, quiet: Bool = false, stopped: StopFlag? = nil) async -> (AVAudioEngine?, String?, String?) {
    guard let uid = uid else { return (nil, nil, nil) }
    var engine: AVAudioEngine?
    var tapInstalled = false
    do {
        let devices = try inputDevices()
        guard let device = devices.first(where: { ($0["uid"] as? String) == uid }),
              var deviceID = device["device_id"] as? AudioDeviceID else {
            throw failure("Selected microphone is unavailable; no other microphone was substituted")
        }
        guard await AVCaptureDevice.requestAccess(for: .audio) else {
            throw failure("Microphone permission denied. Allow microphone access in System Settings")
        }
        let e = AVAudioEngine()
        engine = e
        let node = e.inputNode
        guard let unit = node.audioUnit else { throw failure("Microphone AudioUnit unavailable") }
        let status = AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &deviceID, UInt32(MemoryLayout<AudioDeviceID>.size))
        guard status == noErr else { throw failure("Cannot select microphone: OSStatus \(status)") }
        var actual = AudioDeviceID(0)
        var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        guard AudioUnitGetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &actual, &size) == noErr,
              matchesSelectedInput(actual, uid) else { throw failure("Effective microphone differs from selected input") }
        let inputFormat = node.inputFormat(forBus: 0)
        let outputFormat = node.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0, inputFormat.channelCount > 0,
              let recordingFormat = AVAudioFormat(standardFormatWithSampleRate: inputFormat.sampleRate, channels: inputFormat.channelCount) else {
            throw failure("The selected microphone has no usable audio format")
        }
        emit(["event": "microphone_setup", "input_rate": inputFormat.sampleRate,
              "input_channels": inputFormat.channelCount, "output_rate": outputFormat.sampleRate,
              "output_channels": outputFormat.channelCount, "tap_rate": recordingFormat.sampleRate,
              "tap_channels": recordingFormat.channelCount])
        let previousFrames = mic.health()["written_frames"] as? Int64 ?? 0
        // CurrentDevice changes the hardware format, but the node's output may still
        // describe the previous default input. Configure the tap from the selected
        // hardware, never from that stale client format (which can fail with -10868).
        node.installTap(onBus: 0, bufferSize: 4096, format: recordingFormat) { buffer, time in
            if let copy = copyBuffer(buffer) {
                mic.enqueue(copy, hostSeconds: time.isHostTimeValid ? AVAudioTime.seconds(forHostTime: time.hostTime) : monotonic())
            }
        }
        tapInstalled = true
        e.prepare()
        try e.start()
        // Opening the device is not readiness. Bluetooth can renegotiate after start().
        // Keep this same engine for the meeting; do not open/close a separate probe.
        // Silent buffers are valid: muting is not a capture failure.
        let deadline = monotonic() + 5
        while monotonic() < deadline {
            if stopped?.get() == true { throw failure("Recording cancelled while checking the microphone") }
            let health = mic.health()
            if e.isRunning && (health["written_frames"] as? Int64 ?? 0) > previousFrames {
                actual = 0
                size = UInt32(MemoryLayout<AudioDeviceID>.size)
                guard AudioUnitGetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &actual, &size) == noErr,
                      matchesSelectedInput(actual, uid) else {
                    throw failure("Selected microphone changed during startup")
                }
                return (e, uid, nil)
            }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        throw failure("The selected microphone did not deliver audio within five seconds")
    } catch {
        if tapInstalled { engine?.inputNode.removeTap(onBus: 0) }
        engine?.stop()
        let message = "The selected microphone is not ready. Check its connection and microphone permission, then try again."
        if !quiet { emit(["event": "source_failed", "source": "microphone", "message": message, "diagnostic": error.localizedDescription]) }
        return (nil, nil, message)
    }
}
func startSystemAudio(_ enabled:Bool,_ system:TrackWriter) async -> (SCStream?,SystemOutput?,String?) {
    if enabled {
        do {
            let content=try await SCShareableContent.excludingDesktopWindows(false,onScreenWindowsOnly:true);guard let display=content.displays.first else{throw failure("No display available for system audio capture")}
            let filter=SCContentFilter(display:display,excludingWindows:[]);let config=SCStreamConfiguration();config.width=2;config.height=2;config.minimumFrameInterval=CMTime(value:1,timescale:1);config.showsCursor=false;config.capturesAudio=true;config.excludesCurrentProcessAudio=true;config.sampleRate=48000;config.channelCount=2
            let output=SystemOutput(system);let capture=SCStream(filter:filter,configuration:config,delegate:output);try capture.addStreamOutput(output,type:.audio,sampleHandlerQueue:DispatchQueue(label:"parakeet.system"));try await capture.startCapture();return (capture,output,nil)
        }catch{emit(["event":"source_failed","source":"system","message":error.localizedDescription]);return (nil,nil,"system: "+error.localizedDescription)}
    }
    return (nil,nil,nil)
}

func record(_ o: [String: Any]) async throws -> [String: Any] {
    guard let folder = o["directory"] as? String, let sessionID = o["id"] as? String else { throw failure("Missing recording directory or ID") }
    let directory = URL(fileURLWithPath: folder);try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let origin=monotonic();let mic=TrackWriter(source:"microphone",directory:directory,origin:origin);let system=TrackWriter(source:"system",directory:directory,origin:origin)
    let stop=StopFlag();DispatchQueue.global().async { while let line=readLine() { if line == "stop" { stop.stop();return } };stop.stop() }
    var micInterruptions:[[String:Any]]=[];var engine: AVAudioEngine?;var stream: SCStream?;var systemOutput: SystemOutput?;var startupErrors:[String]=[];var effectiveUID:String?;let requestedUID=o["microphone_uid"] as? String
    let microphoneResult = await startMicrophone(requestedUID, mic, stopped: stop)
    engine = microphoneResult.0
    effectiveUID = microphoneResult.1
    // A requested microphone is mandatory. Never turn a failed two-source request
    // into an apparently started system-only meeting.
    if requestedUID != nil && engine == nil {
        throw failure(microphoneResult.2 ?? "The selected microphone is not ready. Meeting not started")
    }
    if stop.get() {
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        throw failure("Recording cancelled before the meeting started")
    }
    let systemResult = await startSystemAudio(o["system_audio"] as? Bool ?? false, system)
    stream = systemResult.0
    systemOutput = systemResult.1
    if let error = systemResult.2 { startupErrors.append(error) }
    guard engine != nil || stream != nil else{throw failure("No capture source started: "+startupErrors.joined(separator:"; "))}
    emit(["event":"capture_started","requested_microphone_uid":requestedUID as Any? ?? NSNull(),"effective_microphone_uid":effectiveUID as Any? ?? NSNull()])
    let maximum=(o["maximum_seconds"] as? Double) ?? 7200
    var lastHealth=0.0
    while !stop.get() && monotonic()-origin<maximum {
        let elapsed=monotonic()-origin
        if elapsed-lastHealth>=1 {
            var routeHealth: [String: Any] = [:]
            if let active=engine,let requested=requestedUID {
                var actual=AudioDeviceID(0);var size=UInt32(MemoryLayout<AudioDeviceID>.size)
                let status=active.inputNode.audioUnit.map{AudioUnitGetProperty($0,kAudioOutputUnitProperty_CurrentDevice,kAudioUnitScope_Global,0,&actual,&size)} ?? kAudio_ParamError
                let uid=status==noErr ? stringProperty(actual,kAudioDevicePropertyDeviceUID):""
                let matches = status == noErr && matchesSelectedInput(actual, requested)
                routeHealth = ["selected_input_matches": matches, "aggregate_input": matches && uid != requested]
                if !matches || !active.isRunning {
                    // Keep the session: record the gap and wait for the same device. No other input is ever used.
                    let wasRunning=active.isRunning
                    let reason = !matches ? "selected microphone disconnected or the effective input changed" : "microphone audio engine stopped"
                    let composition=aggregateComposition(actual)
                    let activeUIDs=composition == nil ? [] : activeSubDeviceUIDs(actual)
                    active.inputNode.removeTap(onBus:0);active.stop();engine=nil;effectiveUID=nil
                    micInterruptions.append(["atSeconds":elapsed,"reason":reason,"deviceQueryStatus":status,"engineWasRunning":wasRunning,"selectedDeviceMatched":matches,"observedDeviceID":actual,"observedDeviceUID":uid,"observedAggregateComposition":composition as Any? ?? NSNull(),"activeSubDeviceUIDs":activeUIDs])
                    emit(["event":"source_interrupted","source":"microphone","effective_microphone_uid":NSNull(),"message":"Microphone audio stopped or changed; retrying your selected microphone. Some audio may be missing."])
                }
            }
            if engine == nil, let requested=requestedUID, !micInterruptions.isEmpty {
                let present=((try? inputDevices()) ?? []).contains{($0["uid"] as? String)==requested}
                if present {
                    let result=await startMicrophone(requested,mic,quiet:true,stopped:stop)
                    if let resumed=result.0 {
                        engine=resumed;effectiveUID=result.1
                        micInterruptions[micInterruptions.count-1]["resumedAtSeconds"]=monotonic()-origin
                        emit(["event":"source_resumed","source":"microphone","effective_microphone_uid":effectiveUID as Any? ?? NSNull()])
                    }
                }
            }
            var mh=mic.health();mh["engine_running"]=engine?.isRunning ?? false
            for (key, value) in routeHealth { mh[key] = value }
            let sh=system.health();emit(["event":"health","elapsed_seconds":elapsed,"microphone":mh,"system":sh]);lastHealth=elapsed
            if elapsed>=12 && mh["first_buffer_ready"] as? Bool != true && sh["first_buffer_ready"] as? Bool != true {startupErrors.append("No source delivered usable buffers within 12 seconds");break}
        }
        try await Task.sleep(nanoseconds:100_000_000)
    }
    engine?.inputNode.removeTap(onBus:0);engine?.stop();if let stream=stream{do{try await stream.stopCapture()}catch{startupErrors.append(error.localizedDescription)}}
    let micReport=mic.report();let sysReport=system.report();let duration=Int((monotonic()-origin)*1000)
    let report:[String:Any]=["id":sessionID,"sourceAlignment":["microphone":micReport,"system":sysReport],"elapsedDurationMs":duration,"microphoneInterruptions":micInterruptions,"requestedSystemAudio": o["system_audio"] as? Bool ?? false,"requestedMicrophoneUID":requestedUID as Any? ?? NSNull(),"effectiveMicrophoneUID":effectiveUID as Any? ?? NSNull(),"errors":startupErrors+(systemOutput?.errors ?? [])]
    let data=try JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]);try data.write(to:directory.appendingPathComponent("meeting-recording-metadata.json"),options:.atomic)
    let title=(o["title"] as? String) ?? "Meeting";let manifest:[String:Any]=["schema":"com.parakeet.local.capture","schemaVersion":1,"meeting":["id":sessionID,"title":title,"createdAt":o["created_at"] as? String ?? ISO8601DateFormatter().string(from:Date()),"durationMs":duration]]
    try JSONSerialization.data(withJSONObject:manifest,options:[.prettyPrinted,.sortedKeys]).write(to:directory.appendingPathComponent("manifest.json"),options:.atomic)
    return report
}

struct Binding: Decodable {let id:String;let key:String;let modifiers:[String];let enabled:Bool}
final class Hotkeys {
    var port: CFMachPort?;var bindings:[Binding]=[];var down:[Int64:String]=[:]
    let keycodes:[String:Int64]=["home":115,"end":119,"pageup":116,"pagedown":121,"f1":122,"f2":120,"f3":99,"f4":118,"f5":96,"f6":97,"f7":98,"f8":100,"f9":101,"f10":109,"f11":103,"f12":111,"space":49,"escape":53,"a":0,"s":1,"d":2,"f":3,"h":4,"g":5,"z":6,"x":7,"c":8,"v":9,"b":11,"q":12,"w":13,"e":14,"r":15,"y":16,"t":17,"1":18,"2":19,"3":20,"4":21,"6":22,"5":23,"9":25,"7":26,"8":28,"0":29,"o":31,"u":32,"i":34,"p":35,"l":37,"j":38,"k":40,"n":45,"m":46]
    /// Modifiers held alone: keycode and the device-dependent flag bit that tells left from right.
    let modifierKeys:[String:(code:Int64,bit:UInt64)]=["leftcontrol":(59,0x1),"leftshift":(56,0x2),"rightshift":(60,0x4),"leftcommand":(55,0x8),"rightcommand":(54,0x10),"leftoption":(58,0x20),"rightoption":(61,0x40),"rightcontrol":(62,0x2000),"fn":(63,0x800000)]
    func run(path:String) throws {
        bindings=try JSONDecoder().decode([Binding].self,from:Data(contentsOf:URL(fileURLWithPath:path))).filter(\.enabled)
        for b in bindings{guard keycodes[b.key.lowercased()] != nil || modifierKeys[b.key.lowercased()] != nil else{throw failure("Unsupported native shortcut key: "+b.key)}}
        let mask=CGEventMask(1<<CGEventType.keyDown.rawValue | 1<<CGEventType.keyUp.rawValue | 1<<CGEventType.flagsChanged.rawValue)
        let pointer=Unmanaged.passUnretained(self).toOpaque()
        port=CGEvent.tapCreate(tap:.cgSessionEventTap,place:.headInsertEventTap,options:.defaultTap,eventsOfInterest:mask,callback:{_,type,event,pointer in
            guard let pointer=pointer else{return Unmanaged.passUnretained(event)};let this=Unmanaged<Hotkeys>.fromOpaque(pointer).takeUnretainedValue()
            if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {this.down.removeAll();emit(["event":"cancel","reason":"event_tap_disabled"]);if let port=this.port{CGEvent.tapEnable(tap:port,enable:true)};return Unmanaged.passUnretained(event)}
            let code=event.getIntegerValueField(.keyboardEventKeycode)
            if type == .flagsChanged {
                // Modifier-only holds: pressed when this modifier's own bit is set, released when it clears.
                guard let binding=this.bindings.first(where:{this.modifierKeys[$0.key.lowercased()]?.code==code}),let key=this.modifierKeys[binding.key.lowercased()] else{return Unmanaged.passUnretained(event)}
                if event.flags.rawValue & key.bit != 0 {
                    if this.down[code]==nil{this.down[code]=binding.id;do{emit(["event":"down","binding":binding.id,"target":try target()])}catch{emit(["event":"cancel","reason":error.localizedDescription])}}
                } else if let owner=this.down.removeValue(forKey:code){emit(["event":"up","binding":owner])}
                return Unmanaged.passUnretained(event)
            }
            if type == .keyUp, let owner=this.down.removeValue(forKey:code){emit(["event":"up","binding":owner]);return nil}
            if type == .keyDown && code == 53 && !this.down.isEmpty { this.down.removeAll();emit(["event":"cancel","reason":"escape"]);return nil }
            if type == .keyDown {
                if this.down[code] != nil{return nil}
                var modifiers:[String]=[];for (flag,name) in [(CGEventFlags.maskCommand,"cmd"),(.maskControl,"ctrl"),(.maskAlternate,"alt"),(.maskShift,"shift")]{if event.flags.contains(flag){modifiers.append(name)}}
                if let binding=this.bindings.first(where:{this.keycodes[$0.key.lowercased()]==code && $0.modifiers.sorted()==modifiers.sorted()}){this.down[code]=binding.id;do{emit(["event":"down","binding":binding.id,"target":try target()])}catch{emit(["event":"cancel","reason":error.localizedDescription])};return nil}
            };return Unmanaged.passUnretained(event)
        },userInfo:pointer)
        guard let port=port else{throw failure("Cannot install event tap; Input Monitoring/Accessibility permission or a conflicting registration may be blocking it")}
        let source=CFMachPortCreateRunLoopSource(kCFAllocatorDefault,port,0);CFRunLoopAddSource(CFRunLoopGetCurrent(),source,.commonModes);CGEvent.tapEnable(tap:port,enable:true);emit(["event":"hotkeys_ready","active":bindings.count]);CFRunLoopRun()
    }
}
@main struct Bridge {
    static func main() async {
        do {
            let parent=getppid();if parent==1{exit(1)};let watchdog=DispatchSource.makeTimerSource(queue:DispatchQueue.global());watchdog.schedule(deadline:.now()+1,repeating:1);watchdog.setEventHandler{if getppid() != parent{exit(1)}};watchdog.resume();defer{watchdog.cancel()}
            let args=CommandLine.arguments;guard args.count>=2 else{throw failure("Expected devices, permissions, target, insert, record, or hotkeys")}
            switch args[1]{
            case "clipboard-self-test":emit(try clipboardSelfTest())
            case "clipboard-test-child":guard args.count==4 else{throw failure("test child requires private pasteboard and directory")};try clipboardTestChild(board:args[2],directory:args[3])
            case "clipboard-recover":guard args.count==3 else{throw failure("clipboard recovery requires its own directory")};emit(try recoverClipboard(URL(fileURLWithPath:args[2])))
            case "timeline-self-test":emit(timelineSelfTest())
            case "microphone-route-self-test":
                let result=microphoneRouteSelfTest();emit(result)
                if result["passed"] as? Bool != true { exit(1) }
            case "microphone-format-self-test":
                guard args.count == 3 else { throw failure("self-test requires a scratch directory") }
                let result = try microphoneFormatSelfTest(URL(fileURLWithPath: args[2])); emit(result)
                if result["passed"] as? Bool != true { exit(1) }
            case "microphone-stall-self-test":
                guard args.count == 3 else { throw failure("stall test requires a scratch directory") }
                let result = try microphoneStallSelfTest(URL(fileURLWithPath: args[2])); emit(result)
                if result["passed"] as? Bool != true { exit(1) }
            case "devices":emit(["devices":try inputDevices()])
            case "permissions":emit(["microphone_status":AVCaptureDevice.authorizationStatus(for:.audio).rawValue,"screen_capture":CGPreflightScreenCaptureAccess(),"accessibility":AXIsProcessTrusted(),"input_monitoring":CGPreflightListenEventAccess(),"prompts_requested":false])
            case "request-permission":
                guard args.count==3 else{throw failure("request-permission requires accessibility, input_monitoring, screen_recording or microphone")}
                let granted:Bool
                switch args[2]{
                case "accessibility":granted=AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt" as CFString:true] as CFDictionary)
                case "input_monitoring":granted=CGRequestListenEventAccess()
                case "screen_recording":granted=CGRequestScreenCaptureAccess()
                case "microphone":granted=await AVCaptureDevice.requestAccess(for:.audio)
                default:throw failure("Unknown permission kind")
                }
                if !granted {
                    let panes = ["microphone": "Privacy_Microphone", "screen_recording": "Privacy_ScreenCapture", "accessibility": "Privacy_Accessibility", "input_monitoring": "Privacy_ListenEvent"]
                    if let pane = panes[args[2]], let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?" + pane) { NSWorkspace.shared.open(url) }
                }
                emit(["permission":args[2],"granted":granted,"prompt_requested":true])
            case "target":emit(try target(bundle:args.count==3 ? args[2]:nil))
            case "insert":guard args.count==3 else{throw failure("insert requires a JSON file")};let o=try JSONSerialization.jsonObject(with:Data(contentsOf:URL(fileURLWithPath:args[2]))) as! [String:Any];emit(try insert(o))
            case "record":guard args.count==3 else{throw failure("record requires a JSON file")};let o=try JSONSerialization.jsonObject(with:Data(contentsOf:URL(fileURLWithPath:args[2]))) as! [String:Any];emit(["event":"completed","report":try await record(o)])
            case "hotkeys":guard args.count==3 else{throw failure("hotkeys requires a bindings file")};try Hotkeys().run(path:args[2])
            default:throw failure("Unknown bridge command")
            }
        }catch{emit(["event":"failed","message":error.localizedDescription]);exit(1)}
    }
}

/// Simulates a device clock running fast and slow; no frame may be dropped and the timeline must
/// stay within the stated bound of the host clock.
func timelineSelfTest() -> [String: Any] {
    var results: [[String: Any]] = []
    for ppm in [-200.0, 0.0, 200.0] {
        let rate = 48_000.0, framesPerBuffer = 4096.0, seconds = 3600.0
        var first = 100.0, timeline: Int64 = 0, written: Int64 = 0, dropped = 0, gaps: Int64 = 0, drift: Int64 = 0
        var host = first, maxError = 0.0
        let buffers = Int(seconds * rate / framesPerBuffer)
        let timeline_ = Timeline()
        for _ in 0..<buffers {
            let step = timeline_.place(hostSeconds: host, first: &first, rate: rate, timeline: timeline)
            if step.gapFrames > 0 { timeline += step.gapFrames; gaps += step.gapFrames }
            if step.driftFrames > 0 { drift += step.driftFrames }
            timeline += Int64(framesPerBuffer); written += Int64(framesPerBuffer)
            // The device delivers a buffer every framesPerBuffer / (rate * (1 + ppm))) host seconds.
            host += framesPerBuffer / (rate * (1 + ppm / 1_000_000))
            maxError = max(maxError, abs(Double(timeline) / rate - (host - 100.0)))
        }
        if written != Int64(buffers) * Int64(framesPerBuffer) { dropped += 1 }
        results.append(["ppm": ppm, "buffers": buffers, "writtenFrames": written, "droppedBuffers": dropped, "insertedSilenceFrames": gaps, "driftReportedFrames": drift, "maxTimelineErrorSeconds": maxError])
    }
    return ["event": "timeline_self_test", "results": results]
}

/// Deterministic route-identity cases; hardware behavior is checked separately.
func microphoneRouteSelfTest() -> [String: Any] {
    let mic: [String: Any] = ["uid": "selected", "channels-in": 2, "channels-out": 0]
    let output: [String: Any] = ["uid": "output", "channels-in": 0, "channels-out": 2]
    let other: [String: Any] = ["uid": "other", "channels-in": 1, "channels-out": 0]
    let cases: [(String, [String: Any], [String], Bool)] = [
        ("selected input with output-only member", ["subdevices": [mic, output]], ["selected", "output"], true),
        ("member order does not change input", ["subdevices": [output, mic]], ["output", "selected"], true),
        ("other microphone", ["subdevices": [other, output]], ["other", "output"], false),
        ("ambiguous mixed inputs", ["subdevices": [mic, other]], ["selected", "other"], false),
        ("selected member inactive", ["subdevices": [mic, output]], ["output"], false),
        ("missing channel metadata", ["subdevices": [["uid": "selected"]]], ["selected"], false),
        ("empty aggregate", ["subdevices": []], ["selected"], false)
    ]
    let failures = cases.compactMap { name, composition, active, expected in
        aggregateUsesSelectedInput(composition, activeUIDs: active, selectedUID: "selected") == expected ? nil : name
    }
    return ["event": "microphone_route_self_test", "cases": cases.count, "passed": failures.isEmpty, "failures": failures]
}

/// Synthetic buffers exercise the actual writer; never opens a microphone.
func microphoneFormatSelfTest(_ scratch: URL) throws -> [String: Any] {
    let directory = scratch.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let origin = monotonic()
    let writer = TrackWriter(source: "synthetic", directory: directory, origin: origin)
    let formats: [(Double, AVAudioChannelCount)] = [(24000, 1), (48000, 2), (24000, 1), (44100, 2), (48000, 2)]
    for (index, format) in formats.enumerated() {
        let buffer = AVAudioPCMBuffer(pcmFormat: AVAudioFormat(standardFormatWithSampleRate: format.0, channels: format.1)!, frameCapacity: AVAudioFrameCount(format.0 / 10))!
        buffer.frameLength = buffer.frameCapacity
        for channel in 0..<Int(format.1) { for frame in 0..<Int(buffer.frameLength) { buffer.floatChannelData![channel][frame] = Float(index + 1) / 10 } }
        writer.enqueue(buffer, hostSeconds: origin + Double(index) / 10)
    }
    let report = writer.report()
    let file = try AVAudioFile(forReading: writer.path)
    let tail = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 100)!
    file.framePosition = max(0, file.length - 100); try file.read(into: tail)
    let tailPresent = tail.frameLength > 0 && abs(tail.floatChannelData![0][0] - 0.5) < 0.01
    let errors = (report["errors"] as? [String] ?? []).count
    let gaps = (report["gaps"] as? [[String: Any]] ?? []).count
    let passed = errors == 0 && gaps == 0 && abs(file.length - 24000) < 128 && tailPresent && file.processingFormat.channelCount == 2
    return ["event": "microphone_format_self_test", "passed": passed, "formats": formats.count, "frames": file.length, "errors": errors, "gaps": gaps, "finalBufferPresent": tailPresent]
}

/// Real converter/writer regression; synthetic samples only, no audio device opened.
func microphoneStallSelfTest(_ scratch: URL) throws -> [String: Any] {
    var writers: [TrackWriter] = []
    var healthy = 0
    for rate in [24000.0, 48000.0] {
        for stalled in [false, true] {
            let directory = scratch.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let origin = monotonic()
            let writer = TrackWriter(source: "synthetic", directory: directory, origin: origin)
            let format = AVAudioFormat(standardFormatWithSampleRate: rate, channels: 2)!
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(rate / 10))!
            buffer.frameLength = buffer.frameCapacity
            for channel in 0..<2 { for frame in 0..<Int(buffer.frameLength) { buffer.floatChannelData![channel][frame] = 0.25 } }
            writer.enqueue(buffer, hostSeconds: origin)
            if stalled { writers.append(writer) }
            else if (writer.report()["errors"] as? [String] ?? []).isEmpty { healthy += 1 }
        }
    }
    Thread.sleep(forTimeInterval: 3.5)
    var detected = 0
    var preserved = 0
    for writer in writers {
        let before = writer.health()
        let report = writer.report()
        if before["is_receiving"] as? Bool == false,
           (report["errors"] as? [String] ?? []).contains("Source stalled before finalization"),
           writer.health()["is_receiving"] as? Bool == false { detected += 1 }
        let file = try AVAudioFile(forReading: writer.path)
        if file.length == 4800 { preserved += 1 }
    }
    return ["event": "microphone_stall_self_test", "passed": healthy == 2 && detected == 2 && preserved == 2,
            "healthyCases": healthy, "stallsDetected": detected, "completeAudioCases": preserved]
}
