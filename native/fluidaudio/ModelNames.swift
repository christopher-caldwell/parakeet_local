// Local subset: names of the Parakeet TDT Core ML bundle. The upstream file also defines download
// repositories; this runtime never downloads anything.
enum ModelNames {
    enum ASR {
        static let preprocessorFile = "Preprocessor.mlmodelc"
        static let encoderFile = "Encoder.mlmodelc"
        static let decoderFile = "Decoder.mlmodelc"
        static let jointFile = "JointDecision.mlmodelc"
        static let vocabularyFile = "parakeet_vocab.json"
    }
}
