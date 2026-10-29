@preconcurrency import CoreML
import Foundation
import OSLog

/// ASR model version enum
public enum AsrModelVersion: Sendable {
    case v2
    case v3

}

public struct AsrModels: Sendable {

    /// Required model names for ASR
    public let encoder: MLModel
    public let preprocessor: MLModel
    public let decoder: MLModel
    public let joint: MLModel
    public let configuration: MLModelConfiguration
    public let vocabulary: [Int: String]
    public let version: AsrModelVersion

    private static let logger = AppLogger(category: "AsrModels")

    public init(
        encoder: MLModel,
        preprocessor: MLModel,
        decoder: MLModel,
        joint: MLModel,
        configuration: MLModelConfiguration,
        vocabulary: [Int: String],
        version: AsrModelVersion
    ) {
        self.encoder = encoder
        self.preprocessor = preprocessor
        self.decoder = decoder
        self.joint = joint
        self.configuration = configuration
        self.vocabulary = vocabulary
        self.version = version
    }

    public var usesSplitFrontend: Bool {
        true
    }
}

extension AsrModels {
    private typealias Names = ModelNames.ASR

    /// Loads an already installed, checksum-verified bundle from `directory`. Local files only.
    static func load(from directory: URL, version: AsrModelVersion = .v2) async throws -> AsrModels {
        let config = defaultConfiguration()
        func model(_ file: String, _ units: MLComputeUnits) throws -> MLModel {
            let configuration = MLModelConfiguration()
            configuration.allowLowPrecisionAccumulationOnGPU = true
            configuration.computeUnits = units
            let url = directory.appendingPathComponent(file)
            guard FileManager.default.fileExists(atPath: url.path) else {
                throw AsrModelsError.modelNotFound(file, url)
            }
            return try MLModel(contentsOf: url, configuration: configuration)
        }
        return AsrModels(
            encoder: try model(Names.encoderFile, config.computeUnits),
            preprocessor: try model(Names.preprocessorFile, .cpuOnly),
            decoder: try model(Names.decoderFile, config.computeUnits),
            joint: try model(Names.jointFile, config.computeUnits),
            configuration: config,
            vocabulary: try loadVocabulary(from: directory),
            version: version
        )
    }

    private static func loadVocabulary(from directory: URL) throws -> [Int: String] {
        let url = directory.appendingPathComponent(Names.vocabularyFile)
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw AsrModelsError.modelNotFound(Names.vocabularyFile, url)
        }
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: String] ?? [:]
        var vocabulary: [Int: String] = [:]
        for (key, value) in json { if let id = Int(key) { vocabulary[id] = value } }
        guard !vocabulary.isEmpty else { throw AsrModelsError.loadingFailed("Vocabulary parsing failed") }
        return vocabulary
    }

    static func defaultConfiguration() -> MLModelConfiguration {
        let config = MLModelConfiguration()
        config.allowLowPrecisionAccumulationOnGPU = true
        config.computeUnits = .cpuAndNeuralEngine
        return config
    }

    static func optimizedConfiguration(for modelType: ANEOptimizer.ModelType, enableFP16: Bool = true) -> MLModelConfiguration {
        let config = MLModelConfiguration()
        config.allowLowPrecisionAccumulationOnGPU = enableFP16
        config.computeUnits = ANEOptimizer.optimalComputeUnits(for: modelType)
        return config
    }

    static func optimizedPredictionOptions() -> MLPredictionOptions {
        let options = MLPredictionOptions()
        options.outputBackings = [:]
        return options
    }
}

enum AsrModelsError: LocalizedError, Sendable {
    case modelNotFound(String, URL)
    case loadingFailed(String)

    var errorDescription: String? {
        switch self {
        case .modelNotFound(let name, let path): return "ASR model '\(name)' not found at: \(path.path)"
        case .loadingFailed(let reason): return "Failed to load ASR models: \(reason)"
        }
    }
}
