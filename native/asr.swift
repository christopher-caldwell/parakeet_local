import Foundation
import CoreML

/// Parakeet TDT transcription over speech chunks that Rust already cut inside silence.
/// Request: {"task":"asr","directory","audio_f32","chunks":[{"file_start","samples"}]}.
/// Reply: segments with times relative to their chunk, so Rust owns every source offset.
func runAsr(_ o: [String: Any]) async throws -> [String: Any] {
    let directory = URL(fileURLWithPath: o["directory"] as! String)
    let audio = URL(fileURLWithPath: o["audio_f32"] as! String)
    let chunks = o["chunks"] as! [[String: Any]]
    let started = Date()
    let models = try await AsrModels.load(from: directory)
    let manager = AsrManager(config: .default)
    try await manager.initialize(models: models)
    let loaded = Date().timeIntervalSince(started)
    let pcm = try Data(contentsOf: audio, options: .mappedIfSafe)
    guard pcm.count % 4 == 0 else {
        throw NSError(domain: "ParakeetModel", code: 2, userInfo: [NSLocalizedDescriptionKey: "Truncated float32 PCM"])
    }
    let total = pcm.count / 4
    var segments: [[String: Any]] = []
    var decodeSeconds = 0.0
    for (index, chunk) in chunks.enumerated() {
        let start = chunk["file_start"] as! Int
        let count = chunk["samples"] as! Int
        guard start >= 0, count > 0, start + count <= total else {
            throw NSError(domain: "ParakeetModel", code: 3, userInfo: [NSLocalizedDescriptionKey: "Chunk outside PCM file"])
        }
        var samples: [Float] = pcm.withUnsafeBytes { raw in
            Array(raw.bindMemory(to: Float.self)[start..<start + count])
        }
        // The model needs at least one second of input; zero padding adds no speech.
        if samples.count < 16_000 { samples.append(contentsOf: [Float](repeating: 0, count: 16_000 - samples.count)) }
        let began = Date()
        let result = try await manager.transcribe(samples, source: .system)
        decodeSeconds += Date().timeIntervalSince(began)
        for piece in splitSentences(result, chunkMs: count / 16) {
            var item = piece
            item["chunk"] = index
            item["confidence"] = Double(result.confidence)
            segments.append(item)
        }
        if let event = try? JSONSerialization.data(withJSONObject: [
            "event": "inference_progress", "completed_ms": (start + count) / 16, "total_ms": total / 16,
        ] as [String: Any]) {
            FileHandle.standardError.write(event)
            FileHandle.standardError.write(Data([10]))
        }
    }
    var usage = rusage()
    var result: [String: Any] = [
        "segments": segments, "model_load_seconds": loaded, "decode_seconds": decodeSeconds,
        "wall_seconds": Date().timeIntervalSince(started), "compute_units": "cpu_and_neural_engine",
        "implementation": "FluidAudio v0.9.1 ASR subset 915a392a3b214205fcf4860cc3608bde35891d03",
    ]
    if getrusage(RUSAGE_SELF, &usage) == 0 { result["helper_peak_rss_bytes"] = Int64(usage.ru_maxrss) }
    return result
}

/// Splits one chunk's tokens at sentence ends or pauses so speakers can be assigned per sentence.
private func splitSentences(_ result: ASRResult, chunkMs: Int) -> [[String: Any]] {
    func piece(_ text: String, _ start: Double, _ end: Double) -> [String: Any]? {
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return nil }
        let startMs = max(0, Int((start * 1000).rounded()))
        let endMs = min(chunkMs, max(startMs, Int((end * 1000).rounded())))
        return ["text": clean, "start_ms": startMs, "end_ms": endMs]
    }
    guard let timings = result.tokenTimings, !timings.isEmpty else {
        return [piece(result.text, 0, Double(chunkMs) / 1000)].compactMap { $0 }
    }
    var out: [[String: Any]] = []
    var text = ""
    var first = timings[0].startTime
    var last = timings[0].endTime
    for token in timings {
        if !text.isEmpty, token.startTime - last > 1.0, let done = piece(text, first, last) {
            out.append(done)
            text = ""
            first = token.startTime
        }
        if text.isEmpty { first = token.startTime }
        text += token.token.replacingOccurrences(of: "\u{2581}", with: " ")
        last = token.endTime
        if let end = token.token.last, ".?!".contains(end), let done = piece(text, first, last) {
            out.append(done)
            text = ""
        }
    }
    if let done = piece(text, first, last) { out.append(done) }
    return out
}
